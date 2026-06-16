use crate::state::battle::effect::{condition_eval::ConditionEval, condition::Hook, target::Target};

pub const HOOK: Hook = Hook::Dead;

pub fn check(target: Target, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let uids = target.entities(eval.fight, owner_uid);
    let result = uids.contains(&eval.target_uid);
    tracing::info!(owner_uid, target_uid = eval.target_uid, ?uids, result, "Dead condition check");
    result
}
