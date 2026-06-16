//! Inline ally-side `BeAttacked` reactive expansion for enemy-side
//! SKILL emissions nested under boss passive bundles.
//!
//! LIVE attaches these reactives directly to the enemy skill's
//! `act_effect` list instead of emitting them as top-level trigger
//! steps. Rubuska's `31250144` is the canonical shape: one passive
//! execution yields two nested 162 wrappers (AddBuff / BuffDel) with
//! the heal packets emitted inline between them.

use sonettobuf::{ActEffect, FightStep};
use std::collections::HashSet;

use crate::state::battle::{
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, SkillEmitKind, drain_to_fight_steps},
    fight_step::wrap_step,
    passives::{
        collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill,
    },
    skill::{
        PhaseFilter, TriggerState, classification::has_be_attacked_reactive_condition,
        euphoria::resolve_with_euphoria,
    },
    trigger::combat::skill_should_fire,
    types::effects::EffectType,
};

fn team_injury_hits_for_uid(uid: i64, damaged_uids: &[i64]) -> i32 {
    damaged_uids
        .iter()
        .filter(|&&damaged_uid| damaged_uid.signum() == uid.signum())
        .count() as i32
}

fn collect_direct_cross_side_damage_targets(step: &FightStep, enemy_caster_uid: i64) -> Vec<i64> {
    let mut out = Vec::new();
    for effect in &step.act_effect {
        let effect_type = effect.effect_type.unwrap_or(0);
        let is_damage = effect_type == EffectType::Damage as i32
            || effect_type == EffectType::Crit as i32
            || effect_type == EffectType::AdditionalDamage as i32
            || effect_type == EffectType::AdditionalDamageCrit as i32
            || effect_type == EffectType::FixedDamage as i32
            || effect_type == EffectType::OriginDamage as i32
            || effect_type == EffectType::OriginCrit as i32;
        let Some(target_uid) = effect.target_id else {
            continue;
        };
        if !is_damage
            || target_uid <= 0
            || enemy_caster_uid == 0
            || enemy_caster_uid.signum() == target_uid.signum()
            || out.contains(&target_uid)
        {
            continue;
        }
        out.push(target_uid);
    }
    out
}

fn extend_with_buff_granted_passives(
    ctx: &FightContext<'_>,
    owner_uid: i64,
    skill_ids: &mut Vec<i32>,
) {
    for instance in ctx.managers.buff_mgr.get(owner_uid) {
        crate::state::battle::utils::for_each_buff_feature_chain(
            instance.buff_id,
            |act_type, parts| {
                let value_parts: Vec<&str> = match act_type {
                    "AddPassiveSkills" => parts.iter().skip(1).copied().collect(),
                    "AddToTarget" | "AddToTargetNoLimit" | "UseDamageSkillAddToTarget" => {
                        parts.iter().skip(2).copied().collect()
                    }
                    _ => Vec::new(),
                };
                for raw in value_parts {
                    for piece in raw.split(',') {
                        let Ok(skill_id) = piece.trim().parse::<i32>() else {
                            continue;
                        };
                        if skill_id <= 0 {
                            continue;
                        }
                        let resolved_skill_id =
                            resolve_with_euphoria(ctx.fight, owner_uid, skill_id);
                        if !skill_ids.contains(&resolved_skill_id) {
                            skill_ids.push(resolved_skill_id);
                        }
                    }
                }
            },
        );
    }
}

fn split_inline_reactive_effects(skill_id: i32, effects: Vec<ActEffect>) -> Vec<ActEffect> {
    let mut out = Vec::new();
    for effect in effects {
        let Some(step) = effect.fight_step.as_ref() else {
            out.push(effect);
            continue;
        };
        if effect.effect_type != Some(EffectType::FightStep as i32)
            || step.act_id != Some(skill_id)
            || step.act_effect.len() <= 1
        {
            out.push(effect);
            continue;
        }

        for inner in &step.act_effect {
            if inner.effect_type == Some(EffectType::Heal as i32) {
                out.push(inner.clone());
                continue;
            }
            let mut split_step = step.clone();
            split_step.act_effect = vec![inner.clone()];
            out.push(wrap_step(split_step));
        }
    }
    out
}

fn allied_hero_ids(ctx: &FightContext<'_>) -> HashSet<i32> {
    ctx.fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .filter_map(|entity| entity.model_id)
        .collect()
}

pub fn inject_into_enemy_skill_step<F>(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    step: &mut FightStep,
    apply_step: &F,
) where
    F: Fn(&mut FightContext<'_>, &FightStep),
{
    let enemy_caster_uid = step.from_id.unwrap_or(0);
    if enemy_caster_uid >= 0 {
        return;
    }

    let attacked_allies = collect_direct_cross_side_damage_targets(step, enemy_caster_uid);
    if attacked_allies.is_empty() {
        return;
    }
    let primary_target_uid = step.to_id.unwrap_or(0);
    let allied_hero_ids = allied_hero_ids(ctx);

    let mut injected_effects = Vec::new();
    for owner_uid in attacked_allies {
        let mut skill_ids = collected.merged_for(owner_uid);
        extend_with_buff_granted_passives(ctx, owner_uid, &mut skill_ids);
        let event = crate::state::battle::trigger::combat::event_from_step(
            ctx.fight,
            enemy_caster_uid,
            owner_uid,
            step.act_id.unwrap_or(0),
            &step.act_effect,
        );
        let teammate_injury_hits = team_injury_hits_for_uid(owner_uid, &event.damaged_uids);
        let teammate_injury_not_reset = ctx.managers.buff_mgr.teammate_injury_not_reset(owner_uid);
        for skill_id in skill_ids {
            if skill_id >= 530000000 {
                continue;
            }
            let skill_hero_id = skill_id / 10000;
            if !has_be_attacked_reactive_condition(skill_id)
                || !allied_hero_ids.contains(&skill_hero_id)
                || !skill_should_fire(
                    owner_uid,
                    skill_id,
                    &event,
                    teammate_injury_hits,
                    teammate_injury_not_reset,
                )
            {
                continue;
            }

            let trigger_state = TriggerState {
                active_use_skill: false,
                skill_id: 0,
                action_order_index: event.action_order_index,
                used_ex_skill: false,
                teammate_use_ex_skill: false,
                trigger_bullet: false,
                event_driven_only: false,
                be_attacked: event.was_attacked_by_enemy(owner_uid),
                hurt_magic: event.took_mental_damage(owner_uid),
                lost_ex_point: event.lost_expoint(owner_uid),
                hurt_not_restraint: event.dealt_damage(owner_uid),
                hurt_restraint: event.dealt_damage(owner_uid),
                teammate_injury_count: teammate_injury_hits,
                teammate_injury_count_not_reset: teammate_injury_not_reset,
                team_injury_count_round: teammate_injury_hits > 0,
                deleted_buff_ids: event.deleted_buff_ids.clone(),
                active_card_cast_uids: ctx.active_card_cast_uids.clone(),
                bloodpool_max_attacker: Some(ctx.mechanics.bloodtithe.get_max(1)),
                bloodpool_value_attacker: Some(ctx.mechanics.bloodtithe.get_value(1)),
            };
            let inject_record_idx = ctx.mechanics.emission_timeline.record(
                crate::state::battle::emission_timeline::EmissionPhase::AllyReactiveInject,
                owner_uid,
                skill_id,
                event.action_order_index,
                Some(event.skill_id),
                Some(event.caster_uid),
            );
            let Ok(skill_effects) = execute_passive_skill(
                ctx,
                owner_uid,
                owner_uid,
                skill_id,
                &PhaseFilter::combat_with(trigger_state),
            ) else {
                continue;
            };
            let skill_effects = split_inline_reactive_effects(skill_id, skill_effects);
            if skill_effects.is_empty() {
                continue;
            }
            ctx.mechanics
                .emission_timeline
                .mark_produced(inject_record_idx);

            let mut queue = EventQueue::new();
            queue.push(BattleEvent::SkillEmit {
                skill_id,
                from: owner_uid,
                to: owner_uid,
                children: skill_effects
                    .clone()
                    .into_iter()
                    .map(|effect| BattleEvent::SerializedActEffect { effect })
                    .collect(),
                kind: SkillEmitKind::EventTriggered,
            });
            let mut event_ctx = EventContext {
                fight: ctx.fight,
                buff_mgr: &mut ctx.managers.buff_mgr,
                entity_mgr: &mut ctx.managers.entity_mgr,
                bloodtithe: &mut ctx.mechanics.bloodtithe,
            };
            let drained = drain_to_fight_steps(queue.drain(), &mut event_ctx);
            for effect in drained {
                if let Some(fight_step) = effect.fight_step.as_ref() {
                    apply_step(ctx, fight_step);
                }
            }
            injected_effects.extend(skill_effects);
        }

        if owner_uid != primary_target_uid {
            continue;
        }
        for observer_uid in collected.attacker_uids() {
            if observer_uid == owner_uid {
                continue;
            }
            let mut buff_granted_skill_ids = Vec::new();
            extend_with_buff_granted_passives(ctx, observer_uid, &mut buff_granted_skill_ids);
            for skill_id in buff_granted_skill_ids {
                if skill_id >= 530000000 {
                    continue;
                }
                let skill_hero_id = skill_id / 10000;
                if !has_be_attacked_reactive_condition(skill_id)
                    || !allied_hero_ids.contains(&skill_hero_id)
                {
                    continue;
                }
                let trigger_state = TriggerState {
                    active_use_skill: false,
                    skill_id: 0,
                    action_order_index: event.action_order_index,
                    used_ex_skill: false,
                    teammate_use_ex_skill: false,
                    trigger_bullet: false,
                    event_driven_only: false,
                    be_attacked: true,
                    hurt_magic: event.took_mental_damage(owner_uid),
                    lost_ex_point: event.lost_expoint(owner_uid),
                    hurt_not_restraint: false,
                    hurt_restraint: false,
                    teammate_injury_count: teammate_injury_hits,
                    teammate_injury_count_not_reset: ctx
                        .managers
                        .buff_mgr
                        .teammate_injury_not_reset(observer_uid),
                    team_injury_count_round: teammate_injury_hits > 0,
                    deleted_buff_ids: event.deleted_buff_ids.clone(),
                    active_card_cast_uids: ctx.active_card_cast_uids.clone(),
                    bloodpool_max_attacker: Some(ctx.mechanics.bloodtithe.get_max(1)),
                    bloodpool_value_attacker: Some(ctx.mechanics.bloodtithe.get_value(1)),
                };
                let mut skill_effects = execute_passive_skill(
                    ctx,
                    observer_uid,
                    owner_uid,
                    skill_id,
                    &PhaseFilter::combat_with(trigger_state.clone()),
                )
                .unwrap_or_default();
                if skill_effects.is_empty() {
                    skill_effects = execute_passive_skill(
                        ctx,
                        observer_uid,
                        observer_uid,
                        skill_id,
                        &PhaseFilter::combat_with(trigger_state),
                    )
                    .unwrap_or_default();
                }
                let skill_effects = split_inline_reactive_effects(skill_id, skill_effects);
                if skill_effects.is_empty() {
                    continue;
                }

                let mut queue = EventQueue::new();
                queue.push(BattleEvent::SkillEmit {
                    skill_id,
                    from: observer_uid,
                    to: owner_uid,
                    children: skill_effects
                        .clone()
                        .into_iter()
                        .map(|effect| BattleEvent::SerializedActEffect { effect })
                        .collect(),
                    kind: SkillEmitKind::EventTriggered,
                });
                let mut event_ctx = EventContext {
                    fight: ctx.fight,
                    buff_mgr: &mut ctx.managers.buff_mgr,
                    entity_mgr: &mut ctx.managers.entity_mgr,
                    bloodtithe: &mut ctx.mechanics.bloodtithe,
                };
                let drained = drain_to_fight_steps(queue.drain(), &mut event_ctx);
                for effect in drained {
                    if let Some(fight_step) = effect.fight_step.as_ref() {
                        apply_step(ctx, fight_step);
                    }
                }
                injected_effects.extend(skill_effects);
            }
        }
    }

    if !injected_effects.is_empty() {
        step.act_effect.extend(injected_effects);
    }
}
