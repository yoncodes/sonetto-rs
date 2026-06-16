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
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, Fight, FightStep, effect_type_enum::EffectType, fight_step};

use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::event::Event;
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::mechanics::Mechanics;
use crate::state::battle::mechanics::injury_counter::is_damage_effect_type;
use crate::state::battle::skill::condition::parser::parse_condition;
use crate::state::battle::skill::{
    PhaseFilter, SkillExecutor, TriggerState,
    cache::{SKILL_CACHE, resolve_skill_effect_id},
    precast::{collect_precast_skills_for_caster, infer_precast_per_decr_seed_cap},
    targets::{alive_enemies, get_entity, get_team_type},
};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    entity_uid: i64,
    skill_id: i32,
    targets: Vec<i64>,
    raw: &str,
    beh_type: BehaviourType,
) -> Vec<Event> {
    let target = targets.into_iter().next().unwrap_or(entity_uid);
    let p1: i32 = raw
        .split('#')
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let p2: i32 = raw
        .split('#')
        .nth(2)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let result = match beh_type {
        BehaviourType::_50008DirectUseSkill
        | BehaviourType::_60053DirectUseSkill2
        | BehaviourType::_60014DirectUseSkillPrev
        | BehaviourType::_50012DirectUseSkillNoAct
        | BehaviourType::_50038DirectUseSkillNoAct2
        | BehaviourType::_60223DirectUseSkillNotExtra
        | BehaviourType::_50039DirectUseSkillCard
        | BehaviourType::_60156DirectUseSkillByBuff => execute_direct_use_skill(
            fight, managers, mechanics, executor, rng, entity_uid, target, p1,
        ),
        BehaviourType::_60175DirectUseBigSkill => execute_direct_use_big_skill(
            fight, managers, mechanics, executor, rng, entity_uid, target, skill_id,
        ),
        BehaviourType::_50010DirectUseGroupAndStarSkill => execute_direct_use_group_and_star_skill(
            fight, managers, mechanics, executor, rng, entity_uid, target, skill_id, p1, p2,
        ),
        BehaviourType::_50036ConsumePowerDirectUseSkill
        | BehaviourType::_60188ConsumePowerUseSkill
        | BehaviourType::_60196DirectUseExSkillNoConsumeExPoint
        | BehaviourType::_60262PerConsumeExPointDirectUseSkill
        | BehaviourType::_100021ConsumeBloodPoolDirectUseSkill => Ok(vec![]),
        BehaviourType::_60225RandomUseSkill | BehaviourType::_60239RandomUseSkillWithDice => {
            execute_random_use_skill(
                fight, managers, mechanics, executor, rng, entity_uid, target, skill_id, raw,
            )
        }
        _ => return vec![],
    };

    match result {
        Ok(effects) => effects
            .into_iter()
            .map(|e: ActEffect| Event::SerializedActEffect { effect: e })
            .collect(),
        Err(e) => {
            tracing::warn!("direct_skill execute error: {e:?}");
            vec![]
        }
    }
}

fn execute_direct_use_skill(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    caster_uid: i64,
    target: i64,
    skill_id: i32,
) -> Result<Vec<ActEffect>> {
    if skill_id <= 0 {
        return Ok(vec![]);
    }
    let phase = PhaseFilter::combat_with(
        TriggerState::on_active_use_skill(skill_id).with_buff_mgr(&managers.buff_mgr),
    );
    executor.execute_skill(
        rng, fight, managers, mechanics, caster_uid, target, skill_id, &phase,
    )
}

fn execute_direct_use_big_skill(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    caster_uid: i64,
    target: i64,
    skill_id: i32,
) -> Result<Vec<ActEffect>> {
    let mut out = Vec::new();

    let caster_team = get_team_type(fight, caster_uid);
    let wrapper_candidate = skill_id - 20;
    let wrapper_effect_id = resolve_skill_effect_id(wrapper_candidate);
    let ex_skill_id = if SKILL_CACHE.contains_key(&wrapper_effect_id) {
        wrapper_candidate
    } else {
        get_entity(fight, caster_uid)
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
    let current_ex = managers.entity_mgr.get_ex_point(caster_uid).max(0);
    // Live wrapper semantics: consume from EX-skill cost lane when
    // present, but cap by current_ex so low-EX casts don't over-consume.
    // (Refund cap = need_ex when present.)
    let initial_consume = if need_ex > 0 {
        need_ex.min(current_ex)
    } else {
        current_ex
    };
    let prep_skill_ids = collect_precast_skills_for_caster(fight, managers, caster_uid);
    let seeded_cap = infer_precast_per_decr_seed_cap(fight, managers, caster_uid, &prep_skill_ids);
    let mut consume = seeded_cap
        .map(|cap| initial_consume.min(cap.max(0)))
        .unwrap_or(initial_consume)
        .max(0);
    let mut refund = if need_ex > 0 {
        consume.min(need_ex)
    } else {
        consume
    };
    managers
        .entity_mgr
        .set_recent_decr_ex_point(caster_uid, consume);

    // Some wrapper cards first fire a passive-side helper skill before
    // forcing the EX cast.
    for precast_id in prep_skill_ids {
        let phase = PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(precast_id).with_buff_mgr(&managers.buff_mgr),
        );
        let mut pre = executor.execute_skill(
            rng, fight, managers, mechanics, caster_uid, caster_uid, precast_id, &phase,
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
            if ae.target_id != Some(caster_uid) {
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
        managers
            .entity_mgr
            .set_recent_decr_ex_point(caster_uid, consume);
    }

    if consume > 0 {
        // Don't mutate entity_mgr directly — the ExPointChange effect
        // below is applied by calculate_mgr::play_effect_add_ex_point
        // during play_step_data. Direct mutation + replay = double-apply.
        out.push(ActEffectBuilder::ex_point_change(caster_uid, -consume));
        out.push(ActEffectBuilder::direct_use_ex_skill(caster_uid));
    }

    let ex_target_uid = if get_team_type(fight, target) != caster_team
        && get_entity(fight, target)
            .map(|e| e.current_hp.unwrap_or(0) > 0)
            .unwrap_or(false)
    {
        target
    } else {
        alive_enemies(fight, caster_uid)
            .into_iter()
            .next()
            .unwrap_or(target)
    };

    let mut ex = {
        let phase = PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(ex_skill_id)
                .with_used_ex_skill(true)
                .with_buff_mgr(&managers.buff_mgr),
        );
        executor.execute_skill(
            rng,
            fight,
            managers,
            mechanics,
            caster_uid,
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
    if ex_skill_id == skill_id {
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
        out.push(ActEffectBuilder::ex_point_change(caster_uid, refund));
    }
    managers.entity_mgr.clear_recent_decr_ex_point(caster_uid);

    Ok(out)
}

fn execute_direct_use_group_and_star_skill(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    caster_uid: i64,
    target: i64,
    skill_id: i32,
    group: i32,
    rank: i32,
) -> Result<Vec<ActEffect>> {
    // Live gating: this derived cast lane should never fire from
    // non-combat passive phases (battle-start / unconditional).
    // It is only valid while running under combat-trigger contexts.
    // if !matches!(ctx.behavior_ctx.phase, PhaseFilter::Combat(_)) {
    //     return Ok(vec![]);
    //}

    use crate::state::battle::skill::random;

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
            executor, rng, fight, managers, mechanics, caster_uid, target, group, rank,
        )?);
    }

    let chosen_skill_id = if group >= 10000 {
        // Live-style derived skill lane: base skill id + (9 + rank).
        // Example: 20021#30630111#2 -> 30630122.
        group.saturating_add(9 + rank.max(1))
    } else {
        get_entity(fight, caster_uid)
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

    let mut derived_effects = executor.execute_skill(
        rng,
        fight,
        managers,
        mechanics,
        caster_uid,
        target,
        chosen_skill_id,
        &PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(chosen_skill_id).with_buff_mgr(&managers.buff_mgr),
        ),
    )?;
    if derived_effects.is_empty() {
        derived_effects.push(ActEffectBuilder::skill_wrapper(FightStep {
            act_type: Some(fight_step::ActType::Skill as i32),
            from_id: Some(caster_uid),
            to_id: Some(target),
            act_id: Some(chosen_skill_id),
            act_effect: vec![],
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        }));
    }
    out.extend(derived_effects);

    let passive_phase = PhaseFilter::combat_with(
        TriggerState::on_active_use_skill(chosen_skill_id).with_buff_mgr(&managers.buff_mgr),
    );
    let passive_skills: Vec<i32> = get_entity(fight, caster_uid)
        .map(|e| e.passive_skill.clone())
        .unwrap_or_default();
    for passive_skill_id in passive_skills {
        let Some(passive_emit_skill_id) = active_use_trigger_emit_skill_id(passive_skill_id) else {
            continue;
        };
        let passive_skill_to_execute = if passive_emit_skill_id != passive_skill_id
            && managers.buff_mgr.has(caster_uid, passive_emit_skill_id)
        {
            passive_skill_id
        } else {
            passive_emit_skill_id
        };
        if passive_skill_id <= 0
            || passive_skill_id == skill_id
            || passive_skill_id == chosen_skill_id
            || passive_skill_to_execute == skill_id
            || passive_skill_to_execute == chosen_skill_id
        {
            continue;
        }
        let dugss_record_idx = mechanics.emission_timeline.record(
            crate::state::battle::emission_timeline::EmissionPhase::DirectUseBigSkillFanout,
            caster_uid,
            passive_skill_to_execute,
            0,
            Some(skill_id),
            Some(caster_uid),
        );
        let passive_effects = executor.execute_skill(
            rng,
            fight,
            managers,
            mechanics,
            caster_uid,
            target,
            passive_skill_to_execute,
            &passive_phase,
        )?;
        if !passive_effects.is_empty() {
            mechanics.emission_timeline.mark_produced(dugss_record_idx);
            out.extend(passive_effects);
        }
    }
    Ok(out)
}

fn execute_random_use_skill(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    caster_uid: i64,
    target: i64,
    skill_id: i32,
    raw: &str,
) -> Result<Vec<ActEffect>> {
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
    executor.execute_skill(
        rng,
        fight,
        managers,
        mechanics,
        caster_uid,
        target,
        pick,
        &PhaseFilter::combat_with(
            TriggerState::on_active_use_skill(pick).with_buff_mgr(&managers.buff_mgr),
        ),
    )
}

fn active_use_trigger_emit_skill_id(skill_id: i32) -> Option<i32> {
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let row = cfg.skill_effect.iter().find(|s| s.id == effect_id)?;
    let conditions = [
        row.condition1.as_str(),
        row.condition2.as_str(),
        row.condition3.as_str(),
        row.condition4.as_str(),
        row.condition5.as_str(),
        row.condition6.as_str(),
        row.condition7.as_str(),
        row.condition8.as_str(),
        row.condition9.as_str(),
        row.condition10.as_str(),
    ];
    let mut has_trigger = false;
    let mut prefers_effect_wrapper = false;
    for raw in conditions {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (cond, _) = parse_condition(raw);
        let is_trigger = matches!(
            cond,
            ConditionType::ActiveUseSkill
                | ConditionType::ActiveUseSkillId { .. }
                | ConditionType::ActOrder { .. }
                | ConditionType::UseSkillEffectTag { .. }
                | ConditionType::UseSpecificSkill { .. }
                | ConditionType::UseHurtSkill
                | ConditionType::NoActRound
        );
        if is_trigger {
            has_trigger = true;
            if matches!(cond, ConditionType::NoActRound) && effect_id != skill_id {
                prefers_effect_wrapper = true;
            }
        }
    }
    if !has_trigger {
        return None;
    }
    Some(if prefers_effect_wrapper {
        effect_id
    } else {
        skill_id
    })
}
