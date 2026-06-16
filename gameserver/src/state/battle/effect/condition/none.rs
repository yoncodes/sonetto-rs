use crate::state::battle::effect::{condition_eval::ConditionEval, condition::Hook, target::Target};

pub fn check(target: Target, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let uids = target.entities(eval.fight, owner_uid);
    let result = uids.contains(&eval.target_uid);
    result
}
