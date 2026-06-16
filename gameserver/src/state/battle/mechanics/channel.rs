use std::collections::HashSet;

use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    buff_actions::monitor_continue::buff_get_monitor_continue_channel_params,
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::{effect_container_step, wrap_step},
    manager::{buff_mgr::BuffInstance, round_mgr::{apply_step_and_maybe_sync, deleted_buff_ids_from_delta, expand_trigger_chain, first_alive_defender_uid}},
    passives::{
        collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill,
    },
    skill::{PhaseFilter, TriggerState},
    steps::trigger_embed,
    trigger::combat::event_from_step,
    types::effects::EffectType,
};

/// Tracks entities that have MonitorContinueChannel passives.
/// Pre-built at battle start — zero runtime scanning.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ChannelState {
    /// (uid, trigger_skill_id) for every ally with a MonitorContinueChannel passive.
    pub monitor_triggers: Vec<(i64, i32)>,
}

impl ChannelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, fight: &Fight) {
        let cfg = config::configs::get();

        let entities = fight.attacker.iter().flat_map(|a| a.entitys.iter());

        for e in entities {
            let uid = e.uid.unwrap_or(0);

            for passive_id in &e.passive_skill {
                let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == *passive_id) else {
                    continue;
                };

                for beh in [
                    &skill.behavior1,
                    &skill.behavior2,
                    &skill.behavior3,
                    &skill.behavior4,
                    &skill.behavior5,
                ] {
                    if beh.is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = beh.split('#').collect();

                    if !parts.first().map(|v| *v == "1").unwrap_or(false) {
                        continue;
                    }
                    let Some(buff_id) = parts.get(1).and_then(|v| v.parse::<i32>().ok()) else {
                        continue;
                    };

                    let is_attr_from_entity = cfg
                        .skill_buff
                        .iter()
                        .find(|b| b.id == buff_id)
                        .map(|b| {
                            b.features.split('|').any(|entry| {
                                let fp: Vec<&str> = entry.split('#').collect();
                                let act_id: i32 =
                                    fp.first().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                                cfg.buff_act
                                    .iter()
                                    .find(|a| a.id == act_id)
                                    .map(|a| a.r#type == "AttrFromEntity")
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);

                    if is_attr_from_entity {
                        self.monitor_triggers.push((uid, *passive_id));
                    }
                }
            }
        }
    }

    pub fn is_active(&self) -> bool {
        !self.monitor_triggers.is_empty()
    }
}

/// A holder + the parameters the buff_act 1024 feature provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorContinueHolder {
    pub holder_uid: i64,
    pub channel_buff_id: i32,
    pub prerequisite_buff_id: i32,
    pub reactive_skill_id: i32,
    pub layer_counter_buff_id: i32,
}

/// Walk every alive ally and emit one MonitorContinueHolder per ally that
/// (a) has at least one buff with a MonitorContinueChannel feature, and
/// (b) has the prerequisite buff present, and
/// (c) has the layer-counter buff with at least one stack/layer remaining.
pub fn find_eligible_monitor_continue_holders(
    ctx: &FightContext<'_>,
) -> Vec<MonitorContinueHolder> {
    let Some(attacker) = ctx.fight.attacker.as_ref() else {
        return Vec::new();
    };

    let mut holders = Vec::new();
    for entity in attacker
        .entitys
        .iter()
        .chain(attacker.sub_entitys.iter())
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
    {
        let Some(holder_uid) = entity.uid else {
            continue;
        };
        let holder_buffs = ctx.managers.buff_mgr.get(holder_uid);
        for instance in holder_buffs {
            // Buff feature shape: `1024#prereq#enemy_emission_reactive#layer_counter#self_emission_reactive`.
            // The buff_get_monitor_continue_channel_params parser names parts
            // (1, 2, 3, 4) as (prerequisite, monitor_buff, emit_effect, emit_skill).
            // For the SELF-emission path (player card play), the engine
            // executes parts[4] (emit_skill).
            // For the ENEMY-emission path (this function), the engine
            // executes parts[2] (the parser's "monitor_buff" slot, which
            // actually carries the boss-side reactive skill id), and gates
            // on parts[3] as the layer-counter buff. The names in the
            // parser tuple don't match the boss-side semantics — the
            // destructure below remaps them to what this path needs.
            let Some((
                prerequisite_buff_id,
                reactive_skill_id,
                layer_counter_buff_id,
                _self_emission_reactive_skill_id,
            )) = buff_get_monitor_continue_channel_params(instance.buff_id)
            else {
                continue;
            };
            if prerequisite_buff_id > 0
                && !holder_buffs
                    .iter()
                    .any(|buff| buff.buff_id == prerequisite_buff_id)
            {
                continue;
            }
            if !holder_buffs.iter().any(|buff| {
                buff.buff_id == layer_counter_buff_id && buff.layer.max(buff.stacks).max(0) >= 1
            }) {
                continue;
            }
            holders.push(MonitorContinueHolder {
                holder_uid,
                channel_buff_id: instance.buff_id,
                prerequisite_buff_id,
                reactive_skill_id,
                layer_counter_buff_id,
            });
        }
    }

    holders
}

fn monitor_continue_splice_index(effects: &[ActEffect]) -> usize {
    effects
        .iter()
        .rposition(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
        .unwrap_or(effects.len())
}

/// Walk every defender entity's `passive_skill` list, and for each
/// passive skill collect the set of skill ids it directly invokes via
/// `50008#X` (UseSkill) and `60225#X:p&Y:p&Z:p` (RandomUseSkill)
/// behavior entries. The returned set is the act_id filter for
/// `MonitorContinueChannel` reactive grafting: the channel only
/// answers boss skills that a battle-rule passive directly invokes,
/// not deeper sub-emissions or generic broadcasts.
///
/// In data terms: boss `passive_skill: […, 530000745, …]` where
/// `530000745.behavior1 = 50008#530000721` and
/// `530000745.behavior2 = 60225#530000751:100&530000752:100&530000753:100`
/// yields `{530000721, 530000751, 530000752, 530000753}`. The
/// channel reactive then fires only on those four act_ids,
/// matching how the official mechanic text describes the chain.
pub fn gather_boss_invoked_reactive_target_skills(fight: &Fight) -> HashSet<i32> {
    let cfg = config::configs::get();
    let mut invoked: HashSet<i32> = HashSet::new();
    let Some(defender) = fight.defender.as_ref() else {
        return invoked;
    };

    for entity in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
        if entity.uid.unwrap_or(0) >= 0 {
            continue;
        }
        for passive_id in &entity.passive_skill {
            let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == *passive_id) else {
                continue;
            };
            for behavior in [
                skill.behavior1.as_str(),
                skill.behavior2.as_str(),
                skill.behavior3.as_str(),
                skill.behavior4.as_str(),
                skill.behavior5.as_str(),
            ] {
                if let Some(rest) = behavior.strip_prefix("50008#") {
                    let head = rest.split('#').next().unwrap_or("");
                    if let Ok(id) = head.parse::<i32>() {
                        if id > 0 {
                            invoked.insert(id);
                        }
                    }
                } else if let Some(rest) = behavior.strip_prefix("60225#") {
                    for chunk in rest.split('&') {
                        let head = chunk.split(':').next().unwrap_or("");
                        if let Ok(id) = head.parse::<i32>() {
                            if id > 0 {
                                invoked.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }

    invoked
}

fn consume_monitor_continue_layer(
    ctx: &mut FightContext<'_>,
    holder_uid: i64,
    layer_counter_buff_id: i32,
) {
    let Some(buff) = ctx
        .managers
        .buff_mgr
        .get(holder_uid)
        .iter()
        .find(|buff| buff.buff_id == layer_counter_buff_id)
        .cloned()
    else {
        return;
    };

    let new_layer = if buff.layer > 0 {
        buff.layer.saturating_sub(1)
    } else {
        0
    };
    let new_stacks = if buff.layer > 0 {
        buff.stacks
    } else {
        buff.stacks.saturating_sub(1)
    };
    ctx.managers.buff_mgr.add_with_uid(
        holder_uid,
        buff.buff_id,
        buff.from_uid,
        buff.from_skill_id,
        new_stacks,
        new_layer,
        buff.uid,
    );
}

pub(crate) fn build_monitor_continue_channel_embeds(
    ctx: &mut FightContext<'_>,
    root_step: &FightStep,
    host_step: &FightStep,
) -> Vec<ActEffect> {
    let caster_uid = root_step.from_id.unwrap_or(0);
    if caster_uid <= 0 || root_step.act_type != Some(fight_step::ActType::Skill as i32) {
        return Vec::new();
    }

    let event = event_from_step(
        ctx.fight,
        caster_uid,
        root_step.to_id.unwrap_or(0),
        root_step.act_id.unwrap_or(0),
        &root_step.act_effect,
    );
    let target_uid = if root_step.to_id.unwrap_or(0) != 0
        && root_step.to_id.unwrap_or(0).signum() != caster_uid.signum()
    {
        root_step.to_id.unwrap_or(0)
    } else {
        event.primary_target_uid
    };
    if target_uid == 0 {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for instance in ctx.managers.buff_mgr.get(caster_uid).to_vec() {
        let Some((prerequisite_buff_id, _monitor_buff_id, emit_effect_id, emit_skill_id)) =
            buff_get_monitor_continue_channel_params(instance.buff_id)
        else {
            continue;
        };
        if prerequisite_buff_id > 0 && !ctx.managers.buff_mgr.has(caster_uid, prerequisite_buff_id)
        {
            continue;
        }
        if !seen.insert((emit_effect_id, emit_skill_id)) {
            continue;
        }
        let already_present = host_step.act_effect.iter().any(|effect| {
            effect
                .fight_step
                .as_ref()
                .map(|step| {
                    step.act_id == Some(emit_effect_id) || step.act_id == Some(emit_skill_id)
                })
                .unwrap_or(false)
        });
        if already_present {
            continue;
        }

        if let Some(existing) = ctx
            .managers
            .buff_mgr
            .get(caster_uid)
            .iter()
            .find(|b| b.buff_id == emit_effect_id)
        {
            let update_step = effect_container_step(
                caster_uid,
                caster_uid,
                emit_effect_id,
                vec![
                    crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                        caster_uid,
                        existing.from_uid,
                        emit_effect_id,
                        existing.uid,
                        existing.stacks.max(1),
                        existing.layer,
                    ),
                ],
            );
            if update_step.act_type == Some(fight_step::ActType::Effect as i32)
                && update_step.act_effect.len() == 1
                && let Some(effect) = update_step.act_effect.first().cloned()
                && effect.effect_type == Some(162)
            {
                out.push(effect);
            } else {
                out.push(wrap_step(update_step));
            }
        }

        let teammate_injury_hits = event
            .damaged_uids
            .iter()
            .filter(|&&d| d.signum() == caster_uid.signum())
            .count() as i32;
        let trigger_state = TriggerState {
            active_use_skill: true,
            skill_id: root_step.act_id.unwrap_or(0),
            action_order_index: 0,
            used_ex_skill: event.used_ex_skill,
            teammate_use_ex_skill: event.teammate_used_ex_skill(caster_uid),
            trigger_bullet: true,
            event_driven_only: false,
            be_attacked: event.was_attacked_by_enemy(caster_uid),
            hurt_magic: event.took_mental_damage(caster_uid),
            lost_ex_point: event.lost_expoint(caster_uid),
            hurt_not_restraint: event.dealt_damage(caster_uid),
            hurt_restraint: event.dealt_damage(caster_uid),
            teammate_injury_count: teammate_injury_hits,
            teammate_injury_count_not_reset: ctx
                .managers
                .buff_mgr
                .teammate_injury_not_reset(caster_uid),
            team_injury_count_round: teammate_injury_hits > 0,
            deleted_buff_ids: event.deleted_buff_ids.clone(),
            active_card_cast_uids: ctx.active_card_cast_uids.clone(),
            bloodpool_max_attacker: Some(ctx.mechanics.bloodtithe.get_max(1)),
            bloodpool_value_attacker: Some(ctx.mechanics.bloodtithe.get_value(1)),
        };

        let channel_record_idx = ctx.mechanics.emission_timeline.record(
            crate::state::battle::emission_timeline::EmissionPhase::ChannelMechanic,
            caster_uid,
            emit_skill_id,
            0,
            None,
            None,
        );
        let Ok(skill_effects) = execute_passive_skill(
            ctx,
            caster_uid,
            target_uid,
            emit_skill_id,
            &PhaseFilter::combat_with(trigger_state),
        ) else {
            continue;
        };
        if !skill_effects.is_empty() {
            ctx.mechanics
                .emission_timeline
                .mark_produced(channel_record_idx);
        }
        out.extend(
            skill_effects
                .into_iter()
                .filter(|effect| effect.effect_type == Some(162)),
        );
    }

    out
}

/// Inject a MonitorContinueChannel reactive into a single enemy SKILL
/// emission's `act_effect` Vec. Returns silently if no Sentinel-shaped
/// holder is currently eligible, the target skill isn't in the
/// `reactive_target_skills` filter, or the holder's reactive emits
/// nothing.
///
/// This is the per-step injector used by
/// `passives::inject::inject_ally_reactives_into_enemy_subtree`. The
/// walker handles recursion + identifying enemy SKILL slots; this
/// function owns Sentinel's holder lookup, channel-buff wrapping,
/// trigger-chain expansion, splice placement, and layer-counter
/// consumption.
///
/// The `reactive_target_skills` filter is typically the set produced
/// by `gather_boss_invoked_reactive_target_skills(fight)` — the
/// channel only answers boss skills directly invoked by a battle-rule
/// passive (e.g. `530000721/751/752`), not deeper sub-emissions or
/// generic broadcasts.
pub fn inject_monitor_continue_into_enemy_skill_step<F, G>(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    step: &mut FightStep,
    reactive_target_skills: &HashSet<i32>,
    expand_trigger_chain: &F,
    deleted_buff_ids_from_delta: &G,
) where
    F: Fn(&mut FightContext<'_>, &CollectedPassives, &FightStep, &[i32]) -> Vec<FightStep>,
    G: Fn(&[(i64, BuffInstance)], &[(i64, BuffInstance)]) -> Vec<i32>,
{
    if !reactive_target_skills.contains(&step.act_id.unwrap_or(0)) {
        return;
    }

    let Some(holder) = find_eligible_monitor_continue_holders(ctx)
        .into_iter()
        .next()
    else {
        return;
    };
    let enemy_caster_uid = step.from_id.unwrap_or(0);
    let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
    let monitor_record_idx = ctx.mechanics.emission_timeline.record(
        crate::state::battle::emission_timeline::EmissionPhase::ChannelMechanic,
        holder.holder_uid,
        holder.reactive_skill_id,
        0,
        step.act_id,
        Some(enemy_caster_uid),
    );
    let Ok(skill_effects) = execute_passive_skill(
        ctx,
        holder.holder_uid,
        enemy_caster_uid,
        holder.reactive_skill_id,
        &ctx.combat_phase(),
    ) else {
        return;
    };
    if skill_effects.is_empty() {
        return;
    }
    ctx.mechanics
        .emission_timeline
        .mark_produced(monitor_record_idx);
    let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
    let runtime_deleted_buff_ids =
        deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

    let mut monitor_step = effect_container_step(
        holder.holder_uid,
        enemy_caster_uid,
        holder.channel_buff_id,
        skill_effects,
    );
    let expanded_steps =
        expand_trigger_chain(ctx, collected, &monitor_step, &runtime_deleted_buff_ids);
    let mut fallback_nested: Vec<ActEffect> = Vec::new();
    for trigger_step in expanded_steps.into_iter().skip(1) {
        let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
        if !trigger_embed::insert_trigger_into_matching_nested(&mut monitor_step, embedded.clone())
        {
            fallback_nested.push(embedded);
        }
    }
    if !fallback_nested.is_empty() {
        let insert_at = trigger_embed::find_trigger_insert_index(&monitor_step.act_effect);
        monitor_step
            .act_effect
            .splice(insert_at..insert_at, fallback_nested);
    }

    let monitor_wrapper = wrap_step(monitor_step);
    let insert_at = monitor_continue_splice_index(&step.act_effect);
    step.act_effect.insert(insert_at, monitor_wrapper);
    consume_monitor_continue_layer(ctx, holder.holder_uid, holder.layer_counter_buff_id);
}

pub(crate) fn inject_channel_followup_buffs_if_missing(
    ctx: &mut FightContext<'_>,
    collected: &crate::state::battle::passives::collector::CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> bool {
    if ctx.fight.cur_round.unwrap_or(1) != 1 {
        return false;
    }

    let attacker_uids: Vec<i64> = ctx
        .fight
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .filter(|e| e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0)
                .filter_map(|e| e.uid)
                .collect()
        })
        .unwrap_or_default();
    if attacker_uids.is_empty() {
        return false;
    }

    let cfg = config::configs::get();

    let mut channel_seed: Option<(i64, i32, i32, i32, i32)> = None;
    'find_seed: for uid in &attacker_uids {
        for instance in ctx.managers.buff_mgr.get(*uid) {
            let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id) else {
                continue;
            };
            for entry in buff_cfg.features.split('|') {
                let parts: Vec<&str> = entry.split('#').collect();
                let act_id = parts
                    .first()
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                let act_type = cfg
                    .buff_act
                    .iter()
                    .find(|a| a.id == act_id)
                    .map(|a| a.r#type.as_str())
                    .unwrap_or("");
                if act_type != "ConsumeBuffContinueChannel" {
                    continue;
                }
                let Some(extra_skill_id) = parts
                    .get(1)
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .filter(|v| *v > 0)
                else {
                    continue;
                };
                let target_type = parts
                    .get(3)
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                let emit_effect_id = parts
                    .get(4)
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                channel_seed = Some((
                    *uid,
                    instance.buff_id,
                    extra_skill_id,
                    target_type,
                    emit_effect_id,
                ));
                break 'find_seed;
            }
        }
    }
    let Some((caster_uid, channel_buff_id, extra_skill_id, target_type, emit_effect_id)) =
        channel_seed
    else {
        return false;
    };

    let selected_target_uid = first_alive_defender_uid(ctx.fight).unwrap_or(0);
    let target_uid = crate::state::battle::skill::targets::TargetResolver::new(
        ctx.fight,
        caster_uid,
        selected_target_uid,
    )
    .behavior(target_type)
    .resolve()
    .into_iter()
    .next()
    .or_else(|| (selected_target_uid != 0).then_some(selected_target_uid))
    .unwrap_or(caster_uid);

    let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
    let phase = crate::state::battle::skill::PhaseFilter::combat_with(
        crate::state::battle::skill::TriggerState::default()
            .with_round_active_card_cast_uids(&ctx.active_card_cast_uids)
            .with_buff_mgr(&ctx.managers.buff_mgr),
    );
    let Ok(channel_effects) =
        execute_passive_skill(ctx, caster_uid, target_uid, extra_skill_id, &phase)
    else {
        return false;
    };
    if channel_effects.is_empty() {
        return false;
    }

    let mut channel_step =
        effect_container_step(caster_uid, caster_uid, channel_buff_id, channel_effects);
    if channel_buff_id == 31020114 && emit_effect_id > 0 {
        let bullet_embeds =
            build_display_only_consume_channel_embeds(ctx, caster_uid, target_uid, emit_effect_id);
        if !bullet_embeds.is_empty()
            && let Some(nested) = channel_step
                .act_effect
                .iter_mut()
                .find_map(|effect| effect.fight_step.as_mut())
        {
            let insert_at = trigger_embed::find_trigger_insert_index(&nested.act_effect);
            nested
                .act_effect
                .splice(insert_at..insert_at, bullet_embeds);
        }
    }

    if apply_step_and_maybe_sync(ctx, &channel_step, true)
        .is_err()
    {
        return false;
    }

    let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
    let runtime_deleted_buff_ids =
        deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);
    let trigger_steps: Vec<FightStep> = expand_trigger_chain(ctx, collected, &channel_step, &runtime_deleted_buff_ids)
        .into_iter()
        .skip(1)
        .filter(|step| trigger_embed::trigger_step_origin_uid(step) == Some(caster_uid))
        .collect();

    let mut host_step = channel_step.clone();
    let nested_skill_idx = host_step.act_effect.iter().position(|effect| {
        effect.effect_type
            == Some(crate::state::battle::types::effects::EffectType::FightStep as i32)
            && effect
                .fight_step
                .as_ref()
                .map(|step| step.act_id == Some(extra_skill_id))
                .unwrap_or(false)
    });

    if let Some(idx) = nested_skill_idx {
        if let Some(nested) = host_step
            .act_effect
            .get_mut(idx)
            .and_then(|effect| effect.fight_step.as_mut())
        {
            let lopera_channel = channel_buff_id == 31020114;
            let mut fallback_nested: Vec<ActEffect> = Vec::new();
            for trigger_step in trigger_steps {
                for embedded in explode_trigger_step_embeds(trigger_step) {
                    let duplicate_host = embedded
                        .fight_step
                        .as_ref()
                        .map(|step| {
                            step.act_type == Some(fight_step::ActType::Skill as i32)
                                && step.act_id == Some(extra_skill_id)
                                && step.from_id == Some(caster_uid)
                        })
                        .unwrap_or(false);
                    if duplicate_host {
                        continue;
                    }
                    if lopera_channel {
                        fallback_nested.push(embedded);
                        continue;
                    }
                    if !trigger_embed::insert_trigger_into_matching_nested(nested, embedded.clone())
                    {
                        fallback_nested.push(embedded);
                    }
                }
            }
            if !fallback_nested.is_empty() {
                if lopera_channel {
                    fallback_nested.sort_by_key(|effect| {
                        effect
                            .fight_step
                            .as_ref()
                            .map(|step| match step.act_id.unwrap_or(0) {
                                31020151 => 0,
                                433711 => 1,
                                _ => 2,
                            })
                            .unwrap_or(3)
                    });
                    nested.act_effect.extend(fallback_nested);
                } else {
                    let insert_at = trigger_embed::find_trigger_insert_index(&nested.act_effect);
                    nested
                        .act_effect
                        .splice(insert_at..insert_at, fallback_nested);
                }
            }
        }
    } else {
        let embedded_steps: Vec<ActEffect> = trigger_steps
            .into_iter()
            .map(trigger_embed::trigger_step_to_embedded_effect)
            .collect();
        if !embedded_steps.is_empty() {
            let insert_at = trigger_embed::find_trigger_insert_index(&host_step.act_effect);
            host_step
                .act_effect
                .splice(insert_at..insert_at, embedded_steps);
        }
    }

    steps.push(crate::state::battle::round::step_shape::build_effect_step(
        vec![wrap_step(host_step)],
    ));
    true
}

fn build_display_only_consume_channel_embeds(
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    target_uid: i64,
    emit_effect_id: i32,
) -> Vec<ActEffect> {
    let mut shadow_fight = ctx.fight.clone();
    let mut shadow_managers = ctx.managers.clone();
    let mut shadow_mechanics = ctx.mechanics.clone();
    let mut shadow_ctx = FightContext::new(
        &mut shadow_fight,
        &mut shadow_managers,
        &mut shadow_mechanics,
    );
    shadow_ctx.set_round_active_card_cast_uids(&ctx.active_card_cast_uids);
    let phase = PhaseFilter::combat_with(
        shadow_ctx
            .active_use_trigger_state(emit_effect_id)
            .with_buff_mgr(&shadow_ctx.managers.buff_mgr),
    );
    let Ok(skill_effects) = execute_passive_skill(
        &mut shadow_ctx,
        caster_uid,
        target_uid,
        emit_effect_id,
        &phase,
    ) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for effect in skill_effects {
        let Some(step) = effect.fight_step.as_ref() else {
            continue;
        };
        if effect.effect_type
            != Some(crate::state::battle::types::effects::EffectType::FightStep as i32)
            || step.act_type != Some(fight_step::ActType::Skill as i32)
            || step.act_id != Some(emit_effect_id)
        {
            continue;
        }
        let child_effects = step.act_effect.clone();
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            kind: crate::state::battle::event_queue::SkillEmitKind::EventTriggered,
            from: caster_uid,
            to: caster_uid,
            skill_id: emit_effect_id,
            children: child_effects
                .into_iter()
                .map(|effect| BattleEvent::SerializedActEffect { effect })
                .collect(),
        });
        let mut event_ctx = EventContext {
            fight: ctx.fight,
            buff_mgr: &mut ctx.managers.buff_mgr,
            entity_mgr: &mut ctx.managers.entity_mgr,
            bloodtithe: &mut ctx.mechanics.bloodtithe,
        };
        let drained = drain_to_fight_steps(queue.drain(), &mut event_ctx)
            .into_iter()
            .next()
            .expect("event-triggered skill emission should serialize to a single ActEffect");
        out.push(drained);
    }

    if let Some(existing) = ctx
        .managers
        .buff_mgr
        .get(caster_uid)
        .iter()
        .find(|buff| buff.buff_id == emit_effect_id)
    {
        let child_effects = vec![
            crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                caster_uid,
                existing.from_uid,
                emit_effect_id,
                existing.uid,
                existing.stacks.max(1),
                existing.layer,
            ),
        ];
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            kind: crate::state::battle::event_queue::SkillEmitKind::EventTriggered,
            from: caster_uid,
            to: caster_uid,
            skill_id: emit_effect_id,
            children: child_effects
                .into_iter()
                .map(|effect| BattleEvent::SerializedActEffect { effect })
                .collect(),
        });
        let mut event_ctx = EventContext {
            fight: ctx.fight,
            buff_mgr: &mut ctx.managers.buff_mgr,
            entity_mgr: &mut ctx.managers.entity_mgr,
            bloodtithe: &mut ctx.mechanics.bloodtithe,
        };
        let drained = drain_to_fight_steps(queue.drain(), &mut event_ctx)
            .into_iter()
            .next()
            .expect("event-triggered skill emission should serialize to a single ActEffect");
        out.push(drained);
    }

    out
}

fn explode_trigger_step_embeds(trigger_step: FightStep) -> Vec<ActEffect> {
    if trigger_step.act_type == Some(fight_step::ActType::Effect as i32) {
        let exploded: Vec<ActEffect> = trigger_step
            .act_effect
            .iter()
            .filter(|effect| effect.effect_type == Some(162))
            .cloned()
            .collect();
        if !exploded.is_empty() {
            return exploded;
        }
    }
    vec![trigger_embed::trigger_step_to_embedded_effect(trigger_step)]
}
