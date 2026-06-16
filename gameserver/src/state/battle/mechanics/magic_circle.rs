//! Magic-circle mechanic — config lookup helpers and runtime embeds.
//!
//! A magic circle is summoned by an ex skill (`skill_effect.isBigSkill == 1`)
//! that carries a `BehaviorType::AddMagicCircle { circle_id }` behavior slot.
//! The circle row in `data/excel2json/magic_circle.json` advertises its
//! carrier state buff (`selfBuff`) and the skill it fires while active
//! (`selfSkills`). All derivations go through config — no circle id, state
//! buff id, or self-skill id is hardcoded anywhere.

use anyhow::Result;
use config::magic_circle::MagicCircle;
use sonettobuf::{ActEffect, Fight, FightStep, MagicCircleInfo, fight_step};

use crate::state::battle::{
    buff_actions::add_passive_skills::{
        collect_grants_from_active_buffs, collect_grants_from_emitted_buffs,
    },
    buff_actions::{EffectContext, apply_after_buff_add_features},
    context::FightContext,
    event_queue::{
        BattleEvent, EventContext, EventQueue, HostEventAccumulator, drain_to_fight_steps,
    },
    fight_step::ActEffectBuilder,
    hero::HeroId,
    heroes::{semmelweis, tuesday},
    mechanics::bloodtithe::BloodtitheState,
    passives::steps::skill::execute_skill as execute_passive_skill,
    skill::{PhaseFilter, SkillExecutor, TriggerState},
    steps::trigger_embed,
    trigger::combat::event_from_step,
    types::effects::EffectType,
    utils::{find_uid_by_hero_id, for_each_buff_feature_chain},
};

/// Summon a magic circle: emit the `MagicCircleAdd` ActEffect with the
/// circle config + create-uid, plus the carrier state buff (`selfBuff`)
/// the circle config advertises. When the state buff carries a
/// `CureUpByLostHp` feature, LIVE emits paired (BuffAdd, CureUpByLostHp)
/// packets per ally — see Semmelweis circle 100051 / buff 308801312.
///
/// Caller-side: `behavior::magic_circle::MagicCircle::execute` for the
/// `AddMagicCircle { circle_id }` variant.
pub fn add_magic_circle(
    ctx: &mut EffectContext<'_>,
    executor: &mut SkillExecutor,
    fight: &Fight,
    caster_uid: i64,
    circle_id: i32,
) -> Result<Vec<ActEffect>> {
    let circle = config::configs::get().magic_circle.get(circle_id).cloned();
    let round = circle.as_ref().map(|circle| circle.round).unwrap_or(0);
    let mut out = Vec::new();
    if let Some(buff_id) = circle
        .as_ref()
        .and_then(|circle| circle.self_buff.trim().parse::<i32>().ok())
        .filter(|id| *id > 0)
    {
        // self_buff dispatch is feature-driven: the buff's
        // `CureUpByLostHp` feature signals an ally-side aura fanout
        // (Semmelweis Blood Domain pattern) instead of the default
        // self-only application. The fanout body lives in the hero
        // module because she's the only hero running this aura shape
        // today; if another hero adopts the same feature it can be
        // factored out then.
        let mut has_cure_up_by_lost_hp = false;
        for_each_buff_feature_chain(buff_id, |act_type, _| {
            if act_type == "CureUpByLostHp" {
                has_cure_up_by_lost_hp = true;
            }
        });
        if has_cure_up_by_lost_hp {
            out.extend(semmelweis::expand_blood_domain_self_buff_aura(
                fight, caster_uid, buff_id,
            ));
        } else {
            out.push(
                crate::state::battle::fight_step::ActEffectBuilder::buff_add(
                    caster_uid, caster_uid, buff_id, 1,
                ),
            );
        }
    }
    if let Some(buff_id) = circle
        .as_ref()
        .and_then(|circle| circle.enemy_buff.trim().parse::<i32>().ok())
        .filter(|id| *id > 0)
    {
        // enemy_buff dispatch is hero-identity-driven: each hero with
        // an enemy-side aura picks targets differently, so the
        // orchestrator delegates to the creator's hero module. If no
        // hero-specific picker matches, we skip the emission rather
        // than guess — a future circle config that adds enemy_buff
        // without a matching hero hook will surface here as a
        // missing-picker warning.
        let target = pick_enemy_buff_target_for_creator(ctx, fight, caster_uid, circle_id);
        if let Some(enemy_uid) = target {
            let original_target = ctx.target_uid();
            ctx.target = enemy_uid;
            let applied = {
                let mut queue = EventQueue::new();
                queue.push(BattleEvent::BuffApply {
                    target: enemy_uid,
                    buff_id,
                    count: 0,
                    layer: 1,
                    from: caster_uid,
                    from_skill_id: 0,
                    config_effect: None,
                });
                let mut synthetic_fight = Fight::default();
                let mut synthetic_bloodtithe = BloodtitheState::new();
                let mut event_ctx = EventContext {
                    fight: &mut synthetic_fight,
                    buff_mgr: &mut ctx.managers.buff_mgr,
                    entity_mgr: &mut ctx.managers.entity_mgr,
                    bloodtithe: &mut synthetic_bloodtithe,
                };
                drain_to_fight_steps(queue.drain(), &mut event_ctx)
            };
            out.extend(applied);
            out.extend(apply_after_buff_add_features(ctx, executor, buff_id, false));
            ctx.target = original_target;
        }
    }
    out.push(ActEffectBuilder::magic_circle_add(
        caster_uid,
        circle_id,
        MagicCircleInfo {
            magic_circle_id: Some(circle_id),
            round: Some(round),
            create_uid: Some(caster_uid),
            electric_level: Some(0),
            electric_progress: Some(0),
            max_electric_progress: Some(0),
        },
    ));

    Ok(out)
}

/// Dispatch the `enemy_buff` target picker to the circle creator's
/// hero module. Returns `None` when the creator doesn't have a
/// registered picker — emits a one-shot warning per circle id so
/// future circle configs that grow `enemy_buff` are easy to spot.
fn pick_enemy_buff_target_for_creator(
    ctx: &EffectContext<'_>,
    fight: &Fight,
    caster_uid: i64,
    circle_id: i32,
) -> Option<i64> {
    if Some(caster_uid) == find_uid_by_hero_id(fight, HeroId::Tuesday.model_id()) {
        return tuesday::pick_lock_sound_enemy_target(ctx, fight, caster_uid);
    }
    tracing::warn!(
        "magic_circle {} carries enemy_buff but no hero-specific picker is registered \
         for creator uid {} — skipping the buff emission",
        circle_id,
        caster_uid,
    );
    None
}

fn active_circle_row(fight: &sonettobuf::Fight) -> Option<&'static MagicCircle> {
    let circle = fight.magic_circle.as_ref()?;
    let circle_id = circle.magic_circle_id?;
    if circle.round.unwrap_or(0) == 0 {
        return None;
    }
    config::configs::get().magic_circle.get(circle_id)
}

/// Whether `skill_id` is listed as `selfSkills` on any magic circle.
/// Used by trigger/combat to exclude magic-circle emissions from team
/// bloodpool attribution.
pub fn is_magic_circle_self_skill(skill_id: i32) -> bool {
    config::configs::get()
        .magic_circle
        .iter()
        .flat_map(|c| {
            c.self_skills
                .split(['|', ',', ';', '#'])
                .filter_map(|part| part.trim().parse::<i32>().ok())
                .collect::<Vec<_>>()
        })
        .any(|id| id == skill_id)
}

fn magic_circle_aura_state(
    ctx: &FightContext<'_>,
    host_step: &FightStep,
    host_caster_uid: i64,
) -> Option<(i32, i64, TriggerState)> {
    if host_caster_uid == 0 || host_step.act_type != Some(fight_step::ActType::Skill as i32) {
        return None;
    }

    let circle = ctx.fight.magic_circle.as_ref()?;
    let create_uid = circle.create_uid.unwrap_or(0);
    if create_uid == 0 || create_uid.signum() != host_caster_uid.signum() {
        return None;
    }

    let circle_cfg = active_circle_row(ctx.fight)?;
    let self_skill_id = circle_cfg.self_skills.trim().parse::<i32>().ok()?;
    if self_skill_id <= 0 || host_step.act_id == Some(self_skill_id) {
        return None;
    }
    let already_present = host_step.act_effect.iter().any(|effect| {
        effect
            .fight_step
            .as_ref()
            .map(|step| step.act_id == Some(self_skill_id))
            .unwrap_or(false)
    });
    if already_present {
        return None;
    }

    let event = event_from_step(
        ctx.fight,
        host_caster_uid,
        host_step.to_id.unwrap_or(0),
        host_step.act_id.unwrap_or(0),
        &host_step.act_effect,
    );
    let wrapper_host_offensive = host_step.act_type == Some(fight_step::ActType::Skill as i32)
        && host_step.to_id.unwrap_or(0) != 0
        && host_step.to_id.unwrap_or(0).signum() != host_caster_uid.signum()
        && direct_root_skill_child(host_step).is_some();
    if !event.dealt_damage(host_caster_uid) && !wrapper_host_offensive {
        return None;
    }

    let target_uid = if host_step.to_id.unwrap_or(0) != 0
        && host_step.to_id.unwrap_or(0).signum() != host_caster_uid.signum()
    {
        host_step.to_id.unwrap_or(0)
    } else {
        event.primary_target_uid
    };
    let teammate_injury_hits = event
        .damaged_uids
        .iter()
        .filter(|&&d| d.signum() == host_caster_uid.signum())
        .count() as i32;
    Some((
        self_skill_id,
        target_uid,
        TriggerState {
            active_use_skill: true,
            skill_id: host_step.act_id.unwrap_or(0),
            action_order_index: 0,
            used_ex_skill: event.used_ex_skill,
            teammate_use_ex_skill: event.teammate_used_ex_skill(host_caster_uid),
            trigger_bullet: event.triggered_bullet_for(host_caster_uid),
            event_driven_only: false,
            be_attacked: event.was_attacked_by_enemy(host_caster_uid),
            hurt_magic: event.took_mental_damage(host_caster_uid),
            lost_ex_point: event.lost_expoint(host_caster_uid),
            hurt_not_restraint: event.dealt_damage(host_caster_uid),
            hurt_restraint: event.dealt_damage(host_caster_uid),
            teammate_injury_count: teammate_injury_hits,
            teammate_injury_count_not_reset: ctx
                .managers
                .buff_mgr
                .teammate_injury_not_reset(host_caster_uid),
            team_injury_count_round: teammate_injury_hits > 0,
            deleted_buff_ids: event.deleted_buff_ids.clone(),
            active_card_cast_uids: ctx.active_card_cast_uids.clone(),
            bloodpool_max_attacker: Some(ctx.mechanics.bloodtithe.get_max(1)),
            bloodpool_value_attacker: Some(ctx.mechanics.bloodtithe.get_value(1)),
        },
    ))
}

/// Look up the active circle's create_uid (the entity that summoned
/// the circle), or `None` if no circle is active. The aura embedder
/// uses this to scope the active-buff harvest to buffs the creator
/// sourced, so the host's private channel state isn't pulled in.
fn active_circle_create_uid(fight: &sonettobuf::Fight) -> Option<i64> {
    fight
        .magic_circle
        .as_ref()
        .and_then(|c| c.create_uid)
        .filter(|uid| *uid != 0)
}

pub(crate) fn build_magic_circle_self_skill_embeds(
    ctx: &mut FightContext<'_>,
    host_step: &FightStep,
    host_caster_uid: i64,
) -> Vec<ActEffect> {
    let Some((self_skill_id, target_uid, trigger_state)) =
        magic_circle_aura_state(ctx, host_step, host_caster_uid)
    else {
        return Vec::new();
    };
    let phase = PhaseFilter::combat_with(trigger_state.clone());
    let Ok(skill_effects) =
        execute_passive_skill(ctx, host_caster_uid, target_uid, self_skill_id, &phase)
    else {
        return Vec::new();
    };
    let mut followup_skill_ids = collect_grants_from_emitted_buffs(
        ctx.fight,
        &skill_effects,
        host_caster_uid,
        self_skill_id,
    );
    // Scope the active-buff harvest to buffs the circle creator
    // sourced. That's the practical proxy for "the circle's selfBuff
    // grant chain" — Semmelweis's aura grants (sourced by Semmelweis
    // when she's the creator) qualify, the host's own private channel
    // state does not, so we don't have to special-case any specific
    // hero's channel buff.
    if let Some(circle_create_uid) = active_circle_create_uid(ctx.fight) {
        collect_grants_from_active_buffs(
            ctx,
            host_caster_uid,
            |from_uid| from_uid == circle_create_uid,
            self_skill_id,
            &mut followup_skill_ids,
        );
    }

    let mut out: Vec<ActEffect> = skill_effects
        .into_iter()
        .filter(|effect| effect.effect_type == Some(162))
        .collect();

    for skill_id in followup_skill_ids {
        let Ok(skill_effects) =
            execute_passive_skill(ctx, host_caster_uid, target_uid, skill_id, &phase)
        else {
            continue;
        };
        out.extend(
            skill_effects
                .into_iter()
                .filter(|effect| effect.effect_type == Some(162)),
        );
    }

    out
}

fn collect_last_nested_aura_target(
    ctx: &FightContext<'_>,
    effects: &[ActEffect],
    path_prefix: &mut Vec<usize>,
    best: &mut Option<Vec<usize>>,
) {
    for (idx, effect) in effects.iter().enumerate() {
        let Some(child) = effect.fight_step.as_ref() else {
            continue;
        };
        path_prefix.push(idx);
        if child.act_type == Some(fight_step::ActType::Skill as i32)
            && magic_circle_aura_state(ctx, child, child.from_id.unwrap_or(0)).is_some()
        {
            *best = Some(path_prefix.clone());
        }
        collect_last_nested_aura_target(ctx, &child.act_effect, path_prefix, best);
        path_prefix.pop();
    }
}

fn find_last_nested_aura_target(
    ctx: &FightContext<'_>,
    host_step: &FightStep,
) -> Option<Vec<usize>> {
    let add_idx = host_step
        .act_effect
        .iter()
        .position(|effect| effect.effect_type == Some(EffectType::MagicCircleAdd as i32))?;
    let mut best = None;
    let mut path = Vec::new();
    collect_last_nested_aura_target(
        ctx,
        &host_step.act_effect[add_idx + 1..],
        &mut path,
        &mut best,
    );
    best.map(|mut path| {
        if let Some(first) = path.first_mut() {
            *first += add_idx + 1;
        }
        path
    })
}

fn nested_step_ref_at_path<'a>(step: &'a FightStep, path: &[usize]) -> Option<&'a FightStep> {
    let mut current = step;
    for idx in path {
        current = current.act_effect.get(*idx)?.fight_step.as_ref()?;
    }
    Some(current)
}

fn nested_step_mut_at_path<'a>(
    step: &'a mut FightStep,
    path: &[usize],
) -> Option<&'a mut FightStep> {
    let mut current = step;
    for idx in path {
        current = current.act_effect.get_mut(*idx)?.fight_step.as_mut()?;
    }
    Some(current)
}

fn find_direct_root_skill_child(host_step: &FightStep) -> Option<usize> {
    let host_act_id = host_step.act_id?;
    let host_from = host_step.from_id?;
    host_step.act_effect.iter().position(|effect| {
        effect.effect_type == Some(162)
            && effect
                .fight_step
                .as_ref()
                .map(|child| {
                    child.act_type == Some(fight_step::ActType::Skill as i32)
                        && child.act_id == Some(host_act_id)
                        && child.from_id == Some(host_from)
                })
                .unwrap_or(false)
    })
}

fn direct_root_skill_child(host_step: &FightStep) -> Option<&FightStep> {
    host_step
        .act_effect
        .get(find_direct_root_skill_child(host_step)?)
        .and_then(|effect| effect.fight_step.as_ref())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MagicCircleApplyKind {
    /// No embeds were applied.
    None,
    /// Embeds spliced into host_step.act_effect at top level.
    /// All embeds were also pushed into accumulator's direct lane.
    TopLevel,
    /// Embeds spliced into a nested SKILL wrapper's act_effect.
    /// Embeds are NOT in the accumulator (nested-path inserts
    /// can't be checked at host top level — known limitation).
    NestedPath,
}

pub(crate) fn apply_magic_circle_self_skill_embeds(
    ctx: &mut FightContext<'_>,
    host_step: &mut FightStep,
) {
    let mut accumulator = HostEventAccumulator::new();
    let _ = apply_magic_circle_self_skill_embeds_with_accumulator(ctx, host_step, &mut accumulator);
}

pub(crate) fn apply_magic_circle_self_skill_embeds_with_accumulator(
    ctx: &mut FightContext<'_>,
    host_step: &mut FightStep,
    accumulator: &mut HostEventAccumulator,
) -> MagicCircleApplyKind {
    if let Some(path) = find_last_nested_aura_target(ctx, host_step)
        && let Some(target_snapshot) = nested_step_ref_at_path(host_step, &path).cloned()
    {
        let embeds = build_magic_circle_self_skill_embeds(
            ctx,
            &target_snapshot,
            target_snapshot.from_id.unwrap_or(0),
        );
        if !embeds.is_empty()
            && let Some(target_step) = nested_step_mut_at_path(host_step, &path)
        {
            let insert_at = trigger_embed::find_trigger_insert_index(&target_step.act_effect);
            target_step.act_effect.splice(insert_at..insert_at, embeds);
            return MagicCircleApplyKind::NestedPath;
        }
    }

    let direct_root_snapshot = direct_root_skill_child(host_step).cloned();
    if let Some(target_snapshot) = direct_root_snapshot {
        let embeds = build_magic_circle_self_skill_embeds(
            ctx,
            &target_snapshot,
            target_snapshot.from_id.unwrap_or(0),
        );
        if !embeds.is_empty() {
            for effect in embeds.iter().cloned() {
                accumulator.push_direct(BattleEvent::SerializedActEffect { effect });
            }
            let insert_at = trigger_embed::find_trigger_insert_index(&host_step.act_effect);
            host_step.act_effect.splice(insert_at..insert_at, embeds);
            return MagicCircleApplyKind::TopLevel;
        }
    }

    let embeds =
        build_magic_circle_self_skill_embeds(ctx, host_step, host_step.from_id.unwrap_or(0));
    if embeds.is_empty() {
        return MagicCircleApplyKind::None;
    }
    for effect in embeds.iter().cloned() {
        accumulator.push_direct(BattleEvent::SerializedActEffect { effect });
    }
    let insert_at = trigger_embed::find_trigger_insert_index(&host_step.act_effect);
    host_step.act_effect.splice(insert_at..insert_at, embeds);
    MagicCircleApplyKind::TopLevel
}
