use super::{EffectSlot, SkillEffect, behavior, condition};

pub fn parse(effect_id: i32, owner_uid: i64) -> Option<SkillEffect> {
    let cfg = config::configs::get();
    let row = cfg.skill_effect.get(effect_id)?;

    let slots = [
        (&row.condition1,  &row.condition_target1,  &row.behavior1,  &row.behavior_target1,  row.limit1,  row.round_limit1),
        (&row.condition2,  &row.condition_target2,  &row.behavior2,  &row.behavior_target2,  row.limit2,  row.round_limit2),
        (&row.condition3,  &row.condition_target3,  &row.behavior3,  &row.behavior_target3,  row.limit3,  row.round_limit3),
        (&row.condition4,  &row.condition_target4,  &row.behavior4,  &row.behavior_target4,  row.limit4,  row.round_limit4),
        (&row.condition5,  &row.condition_target5,  &row.behavior5,  &row.behavior_target5,  row.limit5,  row.round_limit5),
        (&row.condition6,  &row.condition_target6,  &row.behavior6,  &row.behavior_target6,  row.limit6,  row.round_limit6),
        (&row.condition7,  &row.condition_target7,  &row.behavior7,  &row.behavior_target7,  row.limit7,  row.round_limit7),
        (&row.condition8,  &row.condition_target8,  &row.behavior8,  &row.behavior_target8,  row.limit8,  row.round_limit8),
        (&row.condition9,  &row.condition_target9,  &row.behavior9,  &row.behavior_target9,  row.limit9,  row.round_limit9),
        (&row.condition10, &row.condition_target10, &row.behavior10, &row.behavior_target10, row.limit10, row.round_limit10),
        (&row.condition11, &row.condition_target11, &row.behavior11, &row.behavior_target11, row.limit11, row.round_limit11),
        (&row.condition12, &row.condition_target12, &row.behavior12, &row.behavior_target12, row.limit12, row.round_limit12),
        (&row.condition13, &row.condition_target13, &row.behavior13, &row.behavior_target13, row.limit13, row.round_limit13),
        (&row.condition14, &row.condition_target14, &row.behavior14, &row.behavior_target14, row.limit14, row.round_limit14),
        (&row.condition15, &row.condition_target15, &row.behavior15, &row.behavior_target15, row.limit15, row.round_limit15),
        (&row.condition16, &row.condition_target16, &row.behavior16, &row.behavior_target16, row.limit16, row.round_limit16),
        (&row.condition17, &row.condition_target17, &row.behavior17, &row.behavior_target17, row.limit17, row.round_limit17),
        (&row.condition18, &row.condition_target18, &row.behavior18, &row.behavior_target18, row.limit18, row.round_limit18),
        (&row.condition19, &row.condition_target19, &row.behavior19, &row.behavior_target19, row.limit19, row.round_limit19),
        (&row.condition20, &row.condition_target20, &row.behavior20, &row.behavior_target20, row.limit20, row.round_limit20),
    ];

    let slots = slots.iter()
        .filter_map(|(cond, cond_target, beh, beh_target, limit, round_limit)| {
            if beh.is_empty() { return None; }
            let cond_target_id: i32 = cond_target.parse().ok()?;
            let (conditions, op) = condition::parse(cond, cond_target_id, owner_uid)?;
            let beh_target_id: i32 = beh_target.parse().unwrap_or(0);
            let behaviours = behavior::parse(beh, beh_target_id);
            Some(EffectSlot { op, conditions, behaviours, limit: *limit, round_limit: *round_limit, use_count: 0, round_use_count: 0 })
        })
        .collect();

    Some(SkillEffect { owner_uid, slots })
}
