/// Condition checker for UseSpecificSkill (Condition ID 66210).
/// Matches raw format: `66210#<param1>#<param2>`
///
/// ### Parameter Schema:
/// - `<param1>` (Skill Classification / Category Constraint):
///   - `0`: Any basic incantation of the star rank specified in `<param2>`.
///   - `3`: EX/Ultimate skill (matches `Hook::UseExSkill`).
///   - `4`: Basic incantations generally.
///   - `5`: Rank-specific basic incantation (specific star rank is specified in `<param2>`).
///
/// - `<param2>` (Star Rank Constraint):
///   - `0` (or omitted): Any star rank (no rank constraint).
///   - `1` / `2` / `3`: Match `skill_rank` exactly (1-star, 2-star, or 3-star incantation).
///
/// ### Effect Hook vs. Combat Trigger:
/// At this effect hook level (`Hook::AfterAction`), we only have the actor and target alignments.
/// Thus, this check acts as a hook gate checking target alignment against `eval.target_uid`.
/// Robust skill-specific parameters, star ranks, and classifications are matched downstream in the
/// combat trigger pass (`gameserver/src/state/battle/trigger/combat.rs`) via the `TriggerEvent`.
use crate::state::battle::effect::{condition::Hook, condition_eval::ConditionEval, target::Target};

pub const HOOK: Hook = Hook::AfterAction;

pub fn check(target: Target, _params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> bool {
    let uids = target.entities(eval.fight, owner_uid);
    uids.contains(&eval.target_uid)
}
