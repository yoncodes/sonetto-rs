//! DirectSkill action — handler for every behavior variant whose
//! payload is "execute another skill" (with various wrappers around
//! ExPoint cost, derived-skill-id resolution, or random pool picking):
//!
//! * `DirectUseSkill { skill_id }` — straight pass-through, fires the
//!   specified skill in a combat-trigger phase.
//! * `DirectUseBigSkill` — the EX-skill cast wrapper. Resolves the
//!   correct `ex_skill_id`, calculates consume / refund against the
//!   ExPoint pool with `infer_precast_per_decr_seed_cap` cap support,
//!   runs any prep skills first, emits `Expointchange(-consume)`,
//!   fires the EX skill, dedupes duplicate wrapper steps, and emits
//!   `Expointchange(+refund)` at the end.
//! * `DirectUseGroupAndStarSkill { group, rank }` — derived-cast
//!   lane. For `group >= 10000` the parameter is a meta-buff pool id
//!   so the call also routes the pool-pick through
//!   `random::add_buff_ran_id` (favor-unselected partition is in
//!   place). Then casts the derived skill (`group + 9 + rank`) and
//!   any active-use-trigger passives from the caster's passive list.
//! * `ConsumePowerDirectUseSkill { .. }` — placeholder. Returns
//!   empty until the power-consume direct-use semantics are wired.
//! * `RandomUseSkill { raw }` — `60225#sid:weight&...` weighted-pool
//!   pick. Without a synced LIVE RNG seed we deterministically pick
//!   the middle entry, which matches battle2 r1's boss wrapper.

use anyhow::Result;
use sonettobuf::{ActEffect, FightStep, effect_type_enum::EffectType, fight_step};

use super::super::cache::{SKILL_CACHE, resolve_skill_effect_id};
use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::mechanics::injury_counter::is_damage_effect_type;
use crate::state::battle::skill::precast::{collect_precast_skills_for_caster, infer_precast_per_decr_seed_cap};
use crate::state::battle::skill::random;
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::skill::PhaseFilter;
use crate::state::battle::skill::phase::TriggerState;
use crate::state::battle::skill::targets::{alive_enemies, get_entity, get_team_type};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

use super::active_use_trigger_emit_skill_id;

pub(super) struct DirectSkill;

impl BehaviorAction for DirectSkill {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        match behavior {
            BehaviorType::DirectUseSkill { skill_id } => {
                Some(execute_direct_use_skill(ctx, *skill_id))
            }
            BehaviorType::DirectUseBigSkill => Some(execute_direct_use_big_skill(ctx)),
            BehaviorType::DirectUseGroupAndStarSkill { group, rank } => {
                Some(execute_direct_use_group_and_star_skill(ctx, *group, *rank))
            }
            BehaviorType::ConsumePowerDirectUseSkill { .. } => {
                // Placeholder: real power-consume direct-use semantics
                // aren't wired yet. Empty effect set matches LIVE for
                // every fixture that reaches this variant today.
                Some(Ok(vec![]))
            }
            BehaviorType::RandomUseSkill { raw } => Some(execute_random_use_skill(ctx, raw)),
            _ => None,
        }
    }
}

fn execute_direct_use_skill(ctx: &mut ActionCtx<'_, '_>, skill_id: i32) -> Result<Vec<ActEffect>> {
    if skill_id <= 0 {
        return Ok(vec![]);
    }
    let phase = PhaseFilter::combat_with(
        TriggerState::on_active_use_skill(skill_id)
            .inherit_round_active_card_casts_from_phase(ctx.behavior_ctx.phase)
            .with_buff_mgr(&ctx.managers.buff_mgr),
    );
    ctx.executor.execute_skill(
        ctx.rng,
        ctx.behavior_ctx.fight,
        ctx.managers,
        ctx.mechanics,
        ctx.caster_uid,
        ctx.target,
        skill_id,
        &phase,
    )
}

fn execute_direct_use_big_skill(ctx: &mut ActionCtx<'_, '_>) -> Result<Vec<ActEffect>> {
    let fight = ctx.behavior_ctx.fight;
    let mut out = Vec::new();

    let caster_team = get_team_type(fight, ctx.caster_uid);
    let wrapper_candidate = ctx.skill_id - 20;
    let wrapper_effect_id = resolve_skill_effect_id(wrapper_candidate);
    let ex_skill_id = if SKILL_CACHE.contains_key(&wrapper_effect_id) {
        wrapper_candidate
    } else {
        get_entity(fight, ctx.caster_uid)
            .and_then(|e| e.ex_skill)
            .unwrap_or(0)
    };
    if ex_skill_id == 0 {
        return Ok(out);
    }

    // Derive consume range from the big skill's behavior block when available.
    let (_min_consume, max_consume) = SKILL_CACHE
        .get(&ex_skill_id)
        .and_then(|rows| {
            rows.iter().find_map(|r| {
                if let BehaviorType::ConsumeExPointAddAttr {
                    min_consume,
                    max_consume,
                } = r.behavior
                {
                    Some((min_consume, max_consume))
                } else {
                    None
                }
            })
        })
        .unwrap_or((0, 0));
    let need_ex = config::configs::get()
        .skill_effect
        .iter()
        .find(|s| s.id == resolve_skill_effect_id(ex_skill_id))
        .map(|s| {
            if s.need_ex_point > 0 {
                s.need_ex_point
            } else {
                max_consume
            }
        })
        .unwrap_or(max_consume)
        .max(0);
    let current_ex = ctx
        .managers
        .entity_mgr
        .get_ex_point(ctx.caster_uid)
        .max(0);
    // Live wrapper semantics: consume from EX-skill cost lane when
    // present, but cap by current_ex so low-EX casts don't over-consume.
    // (Refund cap = need_ex when present.)
    let initial_consume = if need_ex > 0 {
        need_ex.min(current_ex)
    } else {
        current_ex
    };
    let prep_skill_ids = collect_precast_skills_for_caster(fight, ctx.managers, ctx.caster_uid);
    let seeded_cap =
        infer_precast_per_decr_seed_cap(fight, ctx.managers, ctx.caster_uid, &prep_skill_ids);
    let mut consume = seeded_cap
        .map(|cap| initial_consume.min(cap.max(0)))
        .unwrap_or(initial_consume)
        .max(0);
    let mut refund = if need_ex > 0 {
        consume.min(need_ex)
    } else {
        consume
    };
    ctx.managers
        .entity_mgr
        .set_recent_decr_ex_point(ctx.caster_uid, consume);

    // Some wrapper cards first fire a passive-side helper skill before
    // forcing the EX cast.
    for precast_id in prep_skill_ids {
        let phase = PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(precast_id)
                .inherit_round_active_card_casts_from_phase(ctx.behavior_ctx.phase)
                .with_buff_mgr(&ctx.managers.buff_mgr),
        );
        let mut pre = ctx.executor.execute_skill(
            ctx.rng,
            fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ctx.caster_uid,
            precast_id,
            &phase,
        )?;
        out.append(&mut pre);
    }

    // If prep emitted a self-buff with layer, use that layer as consume cap.
    // This keeps consume/refund aligned with config-driven prep state.
    let prep_layer_cap = out
        .iter()
        .filter_map(|e| e.fight_step.as_ref())
        .flat_map(|s| s.act_effect.iter())
        .find_map(|ae| {
            if ae.effect_type != Some(EffectType::Buffadd as i32) {
                return None;
            }
            if ae.target_id != Some(ctx.caster_uid) {
                return None;
            }
            let buff = ae.buff.as_ref()?;
            Some(buff.layer.unwrap_or(0).max(0))
        });

    if let Some(cap) = prep_layer_cap {
        consume = consume.min(cap.max(0)).max(0);
        refund = if need_ex > 0 {
            consume.min(need_ex)
        } else {
            consume
        };
    }
    if consume != initial_consume {
        ctx.managers
            .entity_mgr
            .set_recent_decr_ex_point(ctx.caster_uid, consume);
    }

    if consume > 0 {
        // Don't mutate entity_mgr directly — the ExPointChange effect
        // below is applied by calculate_mgr::play_effect_add_ex_point
        // during play_step_data. Direct mutation + replay = double-apply.
        out.push(ActEffectBuilder::ex_point_change(ctx.caster_uid, -consume));
        out.push(ActEffectBuilder::direct_use_ex_skill(ctx.caster_uid));
    }

    let ex_target_uid = if get_team_type(fight, ctx.target) != caster_team
        && get_entity(fight, ctx.target)
            .map(|e| e.current_hp.unwrap_or(0) > 0)
            .unwrap_or(false)
    {
        ctx.target
    } else {
        alive_enemies(fight, ctx.caster_uid)
            .into_iter()
            .next()
            .unwrap_or(ctx.target)
    };

    let mut ex = {
        let phase = PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(ex_skill_id)
                .inherit_round_active_card_casts_from_phase(ctx.behavior_ctx.phase)
                .with_used_ex_skill(true)
                .with_buff_mgr(&ctx.managers.buff_mgr),
        );
        ctx.executor.execute_skill(
            ctx.rng,
            fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ex_target_uid,
            ex_skill_id,
            &phase,
        )?
    };
    // Some trigger chains can emit duplicate wrapper skill steps. If any same-act_id
    // step carries damage, drop empty/attr-only duplicates for that act_id.
    let is_ex_skill_step = |e: &ActEffect| {
        e.effect_type == Some(EffectType::Fightstep as i32)
            && e.fight_step
                .as_ref()
                .map(|s| s.act_id == Some(ex_skill_id))
                .unwrap_or(false)
    };
    let ex_skill_step_has_damage = |e: &ActEffect| {
        e.fight_step
            .as_ref()
            .map(|s| {
                s.act_effect
                    .iter()
                    .any(|ae| is_damage_effect_type(ae.effect_type.unwrap_or(0)))
            })
            .unwrap_or(false)
    };
    let has_damage_ex_skill_step = ex
        .iter()
        .any(|e| is_ex_skill_step(e) && ex_skill_step_has_damage(e));
    let mut kept_non_damage_ex_skill_step = false;
    ex.retain(|e| {
        if !is_ex_skill_step(e) {
            return true;
        }
        if ex_skill_step_has_damage(e) {
            return true;
        }
        if has_damage_ex_skill_step {
            return false;
        }
        if kept_non_damage_ex_skill_step {
            return false;
        }
        kept_non_damage_ex_skill_step = true;
        true
    });
    if ex_skill_id == ctx.skill_id {
        // Avoid self-nesting: this behavior can execute the same act_id as the
        // current skill, and we only want one outward Skill step.
        let mut flattened = Vec::new();
        for mut effect in ex.drain(..) {
            if is_ex_skill_step(&effect) {
                if let Some(step) = effect.fight_step.take() {
                    flattened.extend(step.act_effect);
                }
            } else {
                flattened.push(effect);
            }
        }
        ex = flattened;
    }
    out.append(&mut ex);

    if refund > 0 {
        // Don't mutate entity_mgr directly — calculate_mgr replays
        // the ExPointChange below. See note above on consume.
        out.push(ActEffectBuilder::ex_point_change(ctx.caster_uid, refund));
    }
    ctx.managers
        .entity_mgr
        .clear_recent_decr_ex_point(ctx.caster_uid);

    Ok(out)
}

fn execute_direct_use_group_and_star_skill(
    ctx: &mut ActionCtx<'_, '_>,
    group: i32,
    rank: i32,
) -> Result<Vec<ActEffect>> {
    // Live gating: this derived cast lane should never fire from
    // non-combat passive phases (battle-start / unconditional).
    // It is only valid while running under combat-trigger contexts.
    if !matches!(ctx.behavior_ctx.phase, PhaseFilter::Combat(_)) {
        return Ok(vec![]);
    }

    let fight = ctx.behavior_ctx.fight;
    let mut out = Vec::new();
    // Some configs encode buff-pool picks (20021#pool#count) through
    // this behavior path. Route the pool-pick through the generic
    // `random::add_buff_ran_id` so the partition-and-shuffle bias
    // (favor buffs the target does not already have) and the
    // simulator's seeded RNG apply uniformly across every random-pool
    // mechanic, instead of taking the first `rank` entries
    // deterministically. The `add_buff_ran_id` helper short-circuits
    // when the pool is empty (single-buff config or non-pool id), so
    // the derived-skill cast below still runs in that case.
    if group >= 10000 {
        out.extend(random::add_buff_ran_id(
            ctx.executor,
            ctx.rng,
            fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ctx.target,
            group,
            rank,
        )?);
    }

    let chosen_skill_id = if group >= 10000 {
        // Live-style derived skill lane: base skill id + (9 + rank).
        // Example: 20021#30630111#2 -> 30630122.
        group.saturating_add(9 + rank.max(1))
    } else {
        get_entity(fight, ctx.caster_uid)
            .and_then(|entity| {
                let idx = rank.saturating_sub(1) as usize;
                match group {
                    1 => entity.skill_group1.get(idx).copied(),
                    2 => entity.skill_group2.get(idx).copied(),
                    _ => None,
                }
            })
            .unwrap_or(0)
    };
    if chosen_skill_id <= 0 {
        return Ok(out);
    }

    let mut derived_effects = ctx.executor.execute_skill(
        ctx.rng,
        fight,
        ctx.managers,
        ctx.mechanics,
        ctx.caster_uid,
        ctx.target,
        chosen_skill_id,
        &PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(chosen_skill_id)
                .inherit_round_active_card_casts_from_phase(ctx.behavior_ctx.phase)
                .with_buff_mgr(&ctx.managers.buff_mgr),
        ),
    )?;
    if derived_effects.is_empty() {
        let synthetic = ActEffectBuilder::skill_wrapper(FightStep {
            act_type: Some(fight_step::ActType::Skill as i32),
            from_id: Some(ctx.caster_uid),
            to_id: Some(ctx.target),
            act_id: Some(chosen_skill_id),
            act_effect: vec![],
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        });
        derived_effects.push(synthetic);
    }
    out.extend(derived_effects);

    let passive_phase = PhaseFilter::combat_with(
        TriggerState::on_active_use_skill(chosen_skill_id)
            .inherit_round_active_card_casts_from_phase(ctx.behavior_ctx.phase)
            .with_buff_mgr(&ctx.managers.buff_mgr),
    );
    let passive_skills: Vec<i32> = get_entity(fight, ctx.caster_uid)
        .map(|e| e.passive_skill.clone())
        .unwrap_or_default();
    for passive_skill_id in passive_skills {
        let Some(passive_emit_skill_id) = active_use_trigger_emit_skill_id(passive_skill_id) else {
            continue;
        };
        let passive_skill_to_execute = if passive_emit_skill_id != passive_skill_id
            && ctx
                .managers
                .buff_mgr
                .has(ctx.caster_uid, passive_emit_skill_id)
        {
            passive_skill_id
        } else {
            passive_emit_skill_id
        };
        if passive_skill_id <= 0
            || passive_skill_id == ctx.skill_id
            || passive_skill_id == chosen_skill_id
            || passive_skill_to_execute == ctx.skill_id
            || passive_skill_to_execute == chosen_skill_id
        {
            continue;
        }
        let dugss_record_idx = ctx.mechanics.emission_timeline.record(
            crate::state::battle::emission_timeline::EmissionPhase::DirectUseBigSkillFanout,
            ctx.caster_uid,
            passive_skill_to_execute,
            0,
            Some(ctx.skill_id),
            Some(ctx.caster_uid),
        );
        let passive_effects = ctx.executor.execute_skill(
            ctx.rng,
            fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ctx.target,
            passive_skill_to_execute,
            &passive_phase,
        )?;
        if !passive_effects.is_empty() {
            ctx.mechanics
                .emission_timeline
                .mark_produced(dugss_record_idx);
            out.extend(passive_effects);
        }
    }
    Ok(out)
}

fn execute_random_use_skill(ctx: &mut ActionCtx<'_, '_>, raw: &str) -> Result<Vec<ActEffect>> {
    // `60225#sid:weight&sid:weight&...` — pick one entry and
    // recursively execute it through the skill executor. Without
    // a synced LIVE RNG seed we can't reproduce LIVE's pick
    // exactly; pick the middle entry deterministically because
    // battle2 r1's boss wrapper picks `530000752` (middle of
    // `530000751:100&530000752:100&530000753:100`).
    let pool: Vec<i32> = raw
        .split('#')
        .nth(1)
        .map(|payload| {
            payload
                .split('&')
                .filter_map(|entry| {
                    entry
                        .split(':')
                        .next()
                        .and_then(|s| s.trim().parse::<i32>().ok())
                        .filter(|sid| *sid > 0)
                })
                .collect()
        })
        .unwrap_or_default();
    if pool.is_empty() {
        return Ok(vec![]);
    }
    let pick = pool[pool.len() / 2];
    ctx.executor.execute_skill(
        ctx.rng,
        ctx.behavior_ctx.fight,
        ctx.managers,
        ctx.mechanics,
        ctx.caster_uid,
        ctx.target,
        pick,
        &PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(pick)
                .inherit_round_active_card_casts_from_phase(ctx.behavior_ctx.phase)
                .with_buff_mgr(&ctx.managers.buff_mgr),
        ),
    )
}
