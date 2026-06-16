use crate::state::battle::effect::{condition_eval::ConditionEval, condition::Hook, target::Target};

pub const HOOK: Hook = Hook::EvalActiveSkill;

/// Check whether the skill-eval context corresponds to an EX/Ultimate use
/// that matches the configured `target`. The effect system invokes this
/// on the `EvalSkill` hook; we resolve entity targets using the provided
/// `skill_target_uid` (encoded in `eval.target_uid`).
pub fn check(target: Target, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let uids = target.entities_with_skill_target(eval.fight, owner_uid, eval.target_uid);
    uids.contains(&eval.target_uid)
}
