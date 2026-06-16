use crate::state::battle::effect::{condition_eval::ConditionEval, target::Target};

pub fn check(target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> Option<i32> {
    if params.is_empty() {
        return None;
    }
    let uids = target.entities_with_skill_target(eval.fight, owner_uid, eval.target_uid);
    let mut total_stacks = 0;
    for &uid in &uids {
        total_stacks += eval.buff_mgr.count_buff_ids(uid, params);
    }
    if total_stacks > 0 {
        Some(total_stacks)
    } else {
        None
    }
}
