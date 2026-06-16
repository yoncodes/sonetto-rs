use crate::state::battle::effect::{condition::Hook, condition_eval::ConditionEval, target::Target};

pub const HOOK: Hook = Hook::AfterAction;

pub fn check(target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    if params.is_empty() {
        return false;
    }
    let uids = target.entities(eval.fight, owner_uid);
    // Since we don't have the active card's skill_id in eval (the Hook system
    // doesn't thread it through Hook::AfterAction payload), we fallback to treating
    // the target check as the main gate. The full event evaluation is done
    // by the combat trigger pass (`combat.rs`) which uses the `TriggerEvent`.
    uids.contains(&eval.target_uid)
}
