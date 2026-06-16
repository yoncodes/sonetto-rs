use super::ConditionEval;
use super::ConditionType;
use super::action::Condition;

pub(super) struct ExPoint;

impl Condition for ExPoint {
    fn parse(&self, parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        match cond_type {
            "PerExPoint" => Some(ConditionType::PerExPoint {
                threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            "PerDecrExPoint" => Some(ConditionType::PerDecrExPoint {
                threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            "ExpointMoreThan" => Some(ConditionType::ExpointMoreThan {
                threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            "ExpointLessThan" => Some(ConditionType::ExpointLessThan {
                threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            _ => None,
        }
    }

    fn check(&self, condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        let condition_uid = ctx.resolve_entity_target_uid();
        match condition {
            ConditionType::PerExPoint { threshold } => {
                Some(ctx.entity_mgr.get_ex_point(condition_uid) >= *threshold)
            }
            ConditionType::PerDecrExPoint { threshold } => {
                Some(ctx.entity_mgr.get_recent_decr_ex_point(condition_uid) >= *threshold)
            }
            ConditionType::ExpointMoreThan { threshold } => {
                Some(ctx.entity_mgr.get_ex_point(condition_uid) >= *threshold)
            }
            ConditionType::ExpointLessThan { threshold } => {
                Some(ctx.entity_mgr.get_ex_point(condition_uid) <= *threshold)
            }
            _ => None,
        }
    }
}
