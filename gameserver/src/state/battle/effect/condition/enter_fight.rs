use crate::state::battle::effect::{condition_eval::ConditionEval, condition::Hook, target::Target};

pub const HOOK: Hook = Hook::EnterFight;

pub fn check(target: Target, owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let ents = target.entities(eval.fight, owner_uid);
    tracing::info!("EnterFight owner={} targets={:?} entering={}", owner_uid, ents, eval.target_uid);
    ents.contains(&eval.target_uid)
}
