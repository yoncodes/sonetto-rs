use crate::state::battle::effect::{condition::Hook, condition_eval::ConditionEval, target::Target};

pub const HOOK: Hook = Hook::EvalBeingAttacked;

pub fn check(target: Target, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let uids = target.entities(eval.fight, owner_uid);
    uids.contains(&eval.target_uid)
}
