use sonettobuf::Fight;

use super::super::{
    cache::ResolvedBehavior,
    phase::PhaseFilter,
    targets::TargetResolver,
};
use super::ConditionEval;
use super::{eval_trigger_state_condition, TriggerStateConditionContext, TriggerStateConditionOptions};
use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr},
    mechanics::bloodtithe::BloodtitheState,
    types::condition::ConditionType,
};

pub struct BehaviorConditionCtx<'a> {
    pub fight: &'a Fight,
    pub buff_mgr: &'a BuffMgr,
    pub entity_mgr: &'a EntityMgr,
    pub bloodtithe: &'a BloodtitheState,
    pub caster_uid: i64,
    pub target_uid: i64,
    pub has_trigger_state: bool,
    pub phase: &'a PhaseFilter,
}

/// Returns whether behavior slot `b` passes its condition given the current fight state.
pub fn eval_behavior_condition(ctx: &BehaviorConditionCtx<'_>, b: &ResolvedBehavior) -> bool {
    if let PhaseFilter::Combat(event) = ctx.phase {
        let combat_raw = eval_trigger_state_condition(
            &b.condition,
            TriggerStateConditionContext {
                event,
                owner_uid: ctx.caster_uid,
            },
            TriggerStateConditionOptions {
                include_none: true,
                include_combat_none: true,
                ..Default::default()
            },
        );
        if let Some(raw) = combat_raw {
            return if b.negated { !raw } else { raw };
        }
    }

    let condition_uid = resolve_condition_uid(ctx, b);
    let condition_eval = build_condition_eval(ctx, b);

    let raw = if ctx.has_trigger_state
        && b.condition_target == 103
        && ctx.target_uid != 0
        && ctx.target_uid.signum() != ctx.caster_uid.signum()
        && condition_has_trigger_bullet_and_random(&b.condition)
    {
        condition_eval
            .for_target(condition_uid)
            .check_with_random_target(ctx.target_uid, &b.condition)
    } else {
        condition_eval.for_target(condition_uid).check(&b.condition)
    };

    let raw = apply_no_act_seed_hint(ctx.fight, ctx.caster_uid, condition_uid, &b.condition, raw);

    let raw = if b.condition_target == 103 && b.logic_target == 201 && ctx.target_uid != 0 {
        match &b.condition {
            ConditionType::HasBuffId { .. } => {
                raw || condition_eval.for_target(ctx.target_uid).check(&b.condition)
            }
            ConditionType::NoBuffId { .. } => {
                raw && condition_eval.for_target(ctx.target_uid).check(&b.condition)
            }
            _ => raw,
        }
    } else {
        raw
    };

    let raw = if ctx.target_uid != 0 && ctx.target_uid != condition_uid {
        match &b.condition {
            ConditionType::HasBuffGroup { .. } | ConditionType::NoBuffGroup { .. } => condition_eval
                .for_target(ctx.target_uid)
                .with_condition_target(0)
                .check(&b.condition),
            _ => raw,
        }
    } else {
        raw
    };

    if b.negated { !raw } else { raw }
}

fn resolve_condition_uid(ctx: &BehaviorConditionCtx<'_>, b: &ResolvedBehavior) -> i64 {
    if matches!(
        b.condition,
        ConditionType::TargetIsSelf | ConditionType::TargetIsTeamNoMe
    ) {
        if ctx.target_uid != 0 { ctx.target_uid } else { ctx.caster_uid }
    } else if b.condition_target != 0 {
        TargetResolver::new(ctx.fight, ctx.caster_uid, ctx.target_uid)
            .behavior(b.condition_target)
            .logic(b.logic_target)
            .resolve()
            .into_iter()
            .next()
            .unwrap_or(ctx.caster_uid)
    } else {
        ctx.caster_uid
    }
}

fn build_condition_eval<'a>(
    ctx: &'a BehaviorConditionCtx<'a>,
    b: &ResolvedBehavior,
) -> ConditionEval<'a> {
    let eval = ConditionEval::new(
        ctx.fight,
        ctx.buff_mgr,
        ctx.entity_mgr,
        ctx.bloodtithe,
        ctx.caster_uid,
    )
    .with_trigger_state(ctx.has_trigger_state)
    .with_condition_target(b.condition_target);

    if let PhaseFilter::Combat(event) = ctx.phase {
        eval.with_active_card_cast_uids(&event.active_card_cast_uids)
    } else {
        eval
    }
}

fn condition_has_trigger_bullet_and_random(condition: &ConditionType) -> bool {
    match condition {
        ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => {
            let mut saw_trigger_bullet = false;
            let mut saw_random = false;
            for cond in conds {
                saw_trigger_bullet |= matches!(cond, ConditionType::TriggerBullet);
                saw_random |= matches!(cond, ConditionType::Random { .. });
            }
            saw_trigger_bullet && saw_random
        }
        _ => false,
    }
}

fn apply_no_act_seed_hint(
    fight: &Fight,
    caster_uid: i64,
    condition_uid: i64,
    condition: &ConditionType,
    raw: bool,
) -> bool {
    use super::super::{
        cache::{SKILL_CACHE, resolve_skill_effect_id},
        targets::get_entity,
    };
    use crate::state::battle::types::behavior::BehaviorType;

    if condition_uid != caster_uid {
        return raw;
    }
    let wanted_ids = match condition {
        ConditionType::HasBuffId { buff_ids } => {
            if raw { return true; }
            buff_ids
        }
        ConditionType::NoBuffId { buff_ids } => {
            if !raw { return false; }
            buff_ids
        }
        _ => return raw,
    };

    if wanted_ids.is_empty() {
        return raw;
    }
    let cfg = config::configs::get();
    let Some(entity) = get_entity(fight, caster_uid) else {
        return raw;
    };
    for passive_sid in &entity.passive_skill {
        if *passive_sid <= 0 { continue; }
        let effect_id = resolve_skill_effect_id(*passive_sid);
        let Some(rows) = SKILL_CACHE.get(&effect_id) else { continue; };
        for row in rows {
            if !matches!(row.condition, ConditionType::NoActRound) { continue; }
            let BehaviorType::AddBuff { buff_id, .. } = row.behavior else { continue; };
            if wanted_ids.contains(&buff_id) {
                return matches!(condition, ConditionType::HasBuffId { .. });
            }
            let type_id = cfg
                .skill_buff
                .iter()
                .find(|b| b.id == buff_id)
                .map(|b| b.type_id)
                .unwrap_or(0);
            if type_id > 0 && wanted_ids.contains(&type_id) {
                return matches!(condition, ConditionType::HasBuffId { .. });
            }
        }
    }
    raw
}
