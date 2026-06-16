use crate::state::battle::effect::{condition_eval::ConditionEval, condition::Hook, target::Target};
use crate::state::battle::skill::condition::misc::hero_round_interval_matches;

pub const HOOK: Hook = Hook::RoundStart;

pub fn check(_target: Target, params: &[i32], _owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let period = params.get(0).copied().unwrap_or(0);
    let start_round = params.get(1).copied().unwrap_or(period);
    let cur_round = eval.fight.cur_round.unwrap_or(1);
    let result = hero_round_interval_matches(start_round, period, cur_round);
    tracing::info!(
        cur_round,
        start_round,
        period,
        result,
        "HeroRoundInterval condition check"
    );
    result
}
