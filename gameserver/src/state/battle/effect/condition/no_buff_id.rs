use crate::state::battle::effect::{condition::Hook, condition_eval::ConditionEval, target::Target};

pub const HOOK: Hook = Hook::AfterAction;

pub fn check(target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    if params.is_empty() {
        return true;
    }
    let uids = target.entities_with_skill_target(eval.fight, owner_uid, eval.target_uid);
    uids.iter().all(|&uid| params.iter().all(|&buff_id| !eval.buff_mgr.has_buff(uid, buff_id)))
}
