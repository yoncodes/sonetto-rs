use anyhow::Result;
use rand::rngs::StdRng;

use sonettobuf::{ActEffect, BeginRoundOper, FightStep};

use crate::state::battle::{
    context::FightContext,
    emission_timeline::EmissionPhase,
    fight_step::{ActEffectBuilder, make_skill_step},
    passives::collector::collect,
    passives::steps::skill::execute_skill as execute_passive_skill,
    round::RoundState,
    skill::{
        PhaseFilter, SkillExecutor,
        cache::{SKILL_CACHE, resolve_skill_effect_id},
        collect_precast_skills_for_caster,
        euphoria::{resolve_skill_effect_id_for_entity, resolve_with_euphoria},
        infer_precast_per_decr_seed_cap,
    },
    trigger::combat::{TriggerEvent, skill_should_fire},
    types::{behavior::BehaviorType, effects::EffectType},
};
use crate::state::battle::skill::targets::{first_alive_on_side, resolve_target_fallback};

fn execute_skill_and_apply_pending_summons(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
    phase: &PhaseFilter,
) -> Result<Vec<ActEffect>> {
    let effects = executor.execute_skill(
        rng, &*ctx.fight, ctx.managers, ctx.mechanics, caster_uid, target_uid, skill_id, phase,
    )?;
    executor.apply_pending_summons(ctx.fight, ctx.managers)?;
    Ok(effects)
}

pub(crate) async fn play_card(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    state: &mut RoundState,
    oper: BeginRoundOper,
) -> Result<FightStep> {
    let op_index = state.used_cards.len();
    if let Some(silent_ops) = state.replay_silent_ops.as_ref()
        && silent_ops.get(op_index).copied().unwrap_or(false)
    {
        state.used_cards.push(0);
        return Ok(FightStep::default());
    }

    let raw_target_uid = oper.to_id.unwrap_or(0);

    let card = match state
        .replay_selected_cards
        .as_ref()
        .and_then(|cards| cards.get(op_index).cloned())
        .or_else(|| state.selected_cards.get(op_index).cloned())
    {
        Some(c) => c,
        None => return Ok(FightStep::default()),
    };

    let display_caster_uid = card.uid.unwrap_or(0);
    let is_temp_card = card.temp_card.unwrap_or(false) || display_caster_uid == 0;

    let hero_id = card.hero_id.unwrap_or(0);
    let inferred_model_id = if hero_id == 0 {
        let sid = card.skill_id.unwrap_or(0);
        if sid >= 10000 { sid / 10000 } else { 0 }
    } else {
        hero_id
    };
    let exec_caster_uid = if display_caster_uid > 0 {
        display_caster_uid
    } else {
        ctx.fight
            .attacker
            .as_ref()
            .and_then(|a| {
                a.entitys
                    .iter()
                    .chain(a.sub_entitys.iter())
                    .find(|e| e.model_id == Some(inferred_model_id))
                    .and_then(|e| e.uid)
            })
            .unwrap_or(0)
    };

    let skill_id = card.skill_id.unwrap_or(0);

    let resolved_skill_id = {
        let has_behaviors = SKILL_CACHE
            .get(&skill_id)
            .map(|b| !b.is_empty())
            .unwrap_or(false);
        if !has_behaviors && !is_temp_card {
            let chosen = oper.param3.unwrap_or(0);
            if chosen != 0 {
                tracing::warn!("choice card skill={} -> chosen={}", skill_id, chosen);
                chosen
            } else {
                skill_id
            }
        } else {
            skill_id
        }
    };
    let resolved_skill_id =
        resolve_with_euphoria(ctx.fight, exec_caster_uid, resolved_skill_id);
    let target_uid = if raw_target_uid != 0 {
        resolve_target_fallback(ctx.fight, raw_target_uid)
    } else {
        first_alive_on_side(ctx.fight, false).unwrap_or(raw_target_uid)
    };
    let is_direct_ex_card = !is_temp_card
        && ctx
            .fight
            .attacker
            .as_ref()
            .and_then(|a| {
                a.entitys
                    .iter()
                    .chain(a.sub_entitys.iter())
                    .find(|e| e.uid == Some(exec_caster_uid))
            })
            .and_then(|e| e.ex_skill)
            .map(|ex_skill| ex_skill == resolved_skill_id)
            .unwrap_or(false);
    let action_order_index = state.used_cards.len() as i32 + 1;
    let used_ex_skill = ctx
        .fight
        .attacker
        .as_ref()
        .and_then(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .find(|e| e.uid == Some(exec_caster_uid))
        })
        .and_then(|e| e.ex_skill)
        .map(|ex| ex == resolved_skill_id)
        .unwrap_or(false);
    ctx.mark_round_active_card_cast(exec_caster_uid);
    if used_ex_skill {
        ctx.managers.entity_mgr.set_ex_point(exec_caster_uid, 0);
    }
    let card_cast_record_idx = ctx.mechanics.emission_timeline.record(
        EmissionPhase::CardCast,
        exec_caster_uid,
        resolved_skill_id,
        action_order_index,
        None,
        None,
    );

    let mut raw_skill_effects = if is_direct_ex_card {
        build_direct_ex_card_prefix(executor, rng, ctx, exec_caster_uid, resolved_skill_id)?
    } else {
        Vec::new()
    };
    let mut main_skill_effects = execute_skill_and_apply_pending_summons(
        executor,
        rng,
        ctx,
        exec_caster_uid,
        target_uid,
        resolved_skill_id,
        &PhaseFilter::combat_with(
            ctx.active_use_trigger_state(resolved_skill_id)
                .with_action_order_index(action_order_index)
                .with_used_ex_skill(used_ex_skill)
                .with_buff_mgr(&ctx.managers.buff_mgr),
        ),
    )?;
    if !main_skill_effects.is_empty() {
        ctx.mechanics
            .emission_timeline
            .mark_produced(card_cast_record_idx);
    }
    raw_skill_effects.append(&mut main_skill_effects);
    let mut skill_effects = normalize_skill_effects_for_operation(
        std::mem::take(&mut raw_skill_effects),
        exec_caster_uid,
        resolved_skill_id,
    );
    if is_temp_card && skill_effects.is_empty() {
        for fallback_phase in [PhaseFilter::unconditional(), PhaseFilter::enter_fight()] {
            let retry = execute_skill_and_apply_pending_summons(
                executor,
                rng,
                ctx,
                exec_caster_uid,
                target_uid,
                resolved_skill_id,
                &fallback_phase,
            )?;
            let normalized_retry = normalize_skill_effects_for_operation(
                retry,
                exec_caster_uid,
                resolved_skill_id,
            );
            if !normalized_retry.is_empty() {
                skill_effects = normalized_retry;
                break;
            }
        }
    }
    if is_temp_card && skill_effects.is_empty() {
        let mut fallback = build_temp_direct_bigskill_fallback(
            executor,
            rng,
            ctx,
            exec_caster_uid,
            target_uid,
            resolved_skill_id,
        )?;
        if !fallback.is_empty() {
            fallback.extend(skill_effects);
            skill_effects = fallback;
        }
    }

    let collected = collect(ctx.fight, 0);
    let passive_phase = PhaseFilter::combat_with(
        ctx.active_use_trigger_state(resolved_skill_id)
            .with_action_order_index(action_order_index)
            .with_used_ex_skill(used_ex_skill)
            .with_buff_mgr(&ctx.managers.buff_mgr),
    );
    let use_card_event = TriggerEvent {
        caster_uid: exec_caster_uid,
        skill_id: resolved_skill_id,
        action_order_index,
        primary_target_uid: target_uid,
        used_ex_skill,
        from_wrapper_card: display_caster_uid == 0,
        nested_skill_uses: vec![],
        damaged_uids: vec![],
        cross_side_damaged_uids: vec![],
        mental_damaged_uids: vec![],
        lost_expoint_uids: vec![],
        dealer_uids: vec![],
        deleted_buff_ids: ctx.managers.buff_mgr.step_deleted_buff_ids().to_vec(),
        added_buff_uids: vec![],
        added_buff_ids: vec![],
        trigger_bullet: false,
        bloodpool_gain_by_team: vec![],
        bloodpool_gain_by_skill_team: vec![],
        bloodpool_gain_packets_by_team: vec![],
        synthetic_emission: crate::state::battle::trigger::combat::SyntheticEmissionKind::None,
    };
    if !is_temp_card {
        for passive_skill_id in collected.merged_for(exec_caster_uid) {
            if !skill_should_fire(exec_caster_uid, passive_skill_id, &use_card_event, 0, 0) {
                continue;
            }
            let inline_passive_idx = ctx.mechanics.emission_timeline.record(
                EmissionPhase::CardInlinePassive,
                exec_caster_uid,
                passive_skill_id,
                action_order_index,
                Some(resolved_skill_id),
                Some(exec_caster_uid),
            );
            match execute_passive_skill(
                ctx,
                exec_caster_uid,
                target_uid,
                passive_skill_id,
                &passive_phase,
            ) {
                Ok(effects) if !effects.is_empty() => {
                    ctx.mechanics
                        .emission_timeline
                        .mark_produced(inline_passive_idx);
                    skill_effects.extend(effects);
                }
                _ => {}
            }
        }
    }

    state.used_cards.push(op_index as i32);

    Ok(make_skill_step(
        display_caster_uid,
        target_uid,
        resolved_skill_id,
        op_index as i32,
        skill_effects,
    ))
}

fn build_temp_direct_bigskill_fallback(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    target_uid: i64,
    wrapper_skill_id: i32,
) -> Result<Vec<ActEffect>> {
    if wrapper_skill_id <= 0 {
        return Ok(vec![]);
    }
    let ex_skill_id = wrapper_skill_id - 20;
    if ex_skill_id <= 0 {
        return Ok(vec![]);
    }

    let max_consume = SKILL_CACHE
        .get(&ex_skill_id)
        .and_then(|rows| {
            rows.iter().find_map(|r| {
                if let BehaviorType::ConsumeExPointAddAttr { max_consume, .. } = r.behavior {
                    Some(max_consume)
                } else {
                    None
                }
            })
        })
        .unwrap_or(0)
        .max(0);
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
    let current_ex = ctx.managers.entity_mgr.get_ex_point(caster_uid).max(0);
    let initial_consume = if need_ex > 0 {
        need_ex.min(current_ex)
    } else {
        current_ex
    };

    let mut out = Vec::new();
    let prep_skill_ids = collect_precast_skills_for_caster(ctx.fight, ctx.managers, caster_uid);
    let seeded_cap =
        infer_precast_per_decr_seed_cap(ctx.fight, ctx.managers, caster_uid, &prep_skill_ids);
    let consume = seeded_cap
        .map(|cap| initial_consume.min(cap.max(0)))
        .unwrap_or(initial_consume)
        .max(0);
    let refund = if need_ex > 0 {
        consume.min(need_ex)
    } else {
        consume
    };
    ctx.managers
        .entity_mgr
        .set_recent_decr_ex_point(caster_uid, consume);

    for &prep_id in &prep_skill_ids {
        let mut pre = execute_skill_and_apply_pending_summons(
            executor,
            rng,
            ctx,
            caster_uid,
            caster_uid,
            prep_id,
            &PhaseFilter::combat_with(
                ctx.active_use_trigger_state(prep_id)
                    .with_buff_mgr(&ctx.managers.buff_mgr),
            ),
        )?;
        out.append(&mut pre);
    }

    if consume > 0 {
        out.push(ActEffectBuilder::ex_point_change(caster_uid, -consume));
    }

    let mut main = execute_skill_and_apply_pending_summons(
        executor,
        rng,
        ctx,
        caster_uid,
        target_uid,
        ex_skill_id,
        &PhaseFilter::combat_with(
            ctx.active_use_trigger_state(ex_skill_id)
                .with_buff_mgr(&ctx.managers.buff_mgr),
        ),
    )?;
    out.append(&mut main);

    if refund > 0 {
        out.push(ActEffectBuilder::ex_point_change(caster_uid, refund));
    }

    ctx.managers
        .entity_mgr
        .clear_recent_decr_ex_point(caster_uid);
    Ok(out)
}

fn build_direct_ex_card_prefix(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
    skill_id: i32,
) -> Result<Vec<ActEffect>> {
    let current_ex = ctx.managers.entity_mgr.get_ex_point(caster_uid).max(0);
    if current_ex <= 0 {
        ctx.managers
            .entity_mgr
            .set_recent_decr_ex_point(caster_uid, 0);
        return Ok(vec![]);
    }

    let cfg = config::configs::get();
    let skill_effect_id = resolve_skill_effect_id_for_entity(ctx.fight, caster_uid, skill_id);
    let Some(skill_row) = cfg.skill_effect.iter().find(|s| s.id == skill_effect_id) else {
        ctx.managers
            .entity_mgr
            .set_recent_decr_ex_point(caster_uid, 0);
        return Ok(vec![]);
    };

    let attr_consume = SKILL_CACHE
        .get(&skill_effect_id)
        .and_then(|rows| {
            rows.iter().find_map(|row| {
                if let BehaviorType::ConsumeExPointAddAttr { min_consume, .. } = row.behavior {
                    Some(min_consume.max(0))
                } else {
                    None
                }
            })
        })
        .unwrap_or(0)
        .min(current_ex)
        .max(0);
    let point_cost = {
        let raw_cost = if skill_row.need_ex_point > 0 {
            skill_row.need_ex_point
        } else {
            skill_row.big_skill_point
        };
        if raw_cost > 0 {
            raw_cost.max(0)
        } else if attr_consume > 0 {
            0
        } else {
            current_ex
        }
    };

    let prep_skill_ids = collect_precast_skills_for_caster(ctx.fight, ctx.managers, caster_uid);
    let mut out = Vec::new();

    if attr_consume > 0 {
        ctx.managers
            .entity_mgr
            .set_recent_decr_ex_point(caster_uid, attr_consume);
        for &prep_id in &prep_skill_ids {
            let mut pre = execute_skill_and_apply_pending_summons(
                executor,
                rng,
                ctx,
                caster_uid,
                caster_uid,
                prep_id,
                &PhaseFilter::combat_with(
                    ctx.active_use_trigger_state(prep_id)
                        .with_buff_mgr(&ctx.managers.buff_mgr),
                ),
            )?;
            out.append(&mut pre);
        }
        out.push(ActEffectBuilder::ex_point_change(caster_uid, -attr_consume));
    }

    let point_cost = point_cost.min(current_ex).max(0);
    if point_cost > 0 {
        ctx.managers
            .entity_mgr
            .set_recent_decr_ex_point(caster_uid, point_cost);
        for &prep_id in &prep_skill_ids {
            let mut pre = execute_skill_and_apply_pending_summons(
                executor,
                rng,
                ctx,
                caster_uid,
                caster_uid,
                prep_id,
                &PhaseFilter::combat_with(
                    ctx.active_use_trigger_state(prep_id)
                        .with_buff_mgr(&ctx.managers.buff_mgr),
                ),
            )?;
            out.append(&mut pre);
        }
        out.push(ActEffectBuilder::ex_point_change(caster_uid, -point_cost));
    }

    ctx.managers
        .entity_mgr
        .set_recent_decr_ex_point(caster_uid, attr_consume);
    Ok(out)
}

fn normalize_skill_effects_for_operation(
    effects: Vec<ActEffect>,
    caster_uid: i64,
    skill_id: i32,
) -> Vec<ActEffect> {
    if effects.is_empty() {
        return effects;
    }

    let mut out = Vec::new();
    let mut iter = effects.into_iter();

    if let Some(mut first) = iter.next() {
        let first_fight_step = first.fight_step.take();
        let inline_root = first_fight_step.as_ref().is_some_and(|step| {
            first.effect_type == Some(EffectType::FightStep as i32)
                && step.act_type == Some(sonettobuf::fight_step::ActType::Skill as i32)
                && step.act_id == Some(skill_id)
                && (step.from_id == Some(caster_uid) || step.from_id == Some(0))
        });
        if inline_root {
            let step = first_fight_step.expect("inline_root checked first_fight_step presence");
            out.extend(step.act_effect);
        } else {
            out.push(first);
        }
    }

    out.extend(iter);
    out
}
