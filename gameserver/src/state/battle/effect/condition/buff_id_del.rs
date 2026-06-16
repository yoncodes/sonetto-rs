use crate::state::battle::effect::{condition::Hook, condition_eval::ConditionEval, target::Target};

pub const HOOK: Hook = Hook::BuffLost;

pub fn check(target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let Some(lost) = eval.lost_buff_id else { return false; };
    if params.is_empty() { return false; }
    let uids = target.entities(eval.fight, owner_uid);
    uids.iter().any(|&uid| uid == eval.target_uid && params.iter().any(|&p| p == lost))
}
