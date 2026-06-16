use crate::state::battle::fight_step::ActEffectBuilder;

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

pub(super) struct ShieldParams {
    pub target_uid: i64,
    pub amount: i32,
}

pub(super) struct ShieldHandler;

impl BuffActionHandler for ShieldHandler {
    type Params = ShieldParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Shield" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        let permille = parts
            .get(3)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let max_hp: i32 = ctx
            .effect_ctx
            .target_entity()
            .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
            .unwrap_or(0);
        ShieldParams {
            target_uid: ctx.effect_ctx.target_uid(),
            amount: (max_hp as f32 * permille as f32 / 1000.0).ceil() as i32,
        }
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        ActionResult::single(ActEffectBuilder::shield(params.target_uid, params.amount))
    }
}
