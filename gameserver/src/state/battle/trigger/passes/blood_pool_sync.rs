use sonettobuf::FightStep;

use crate::state::battle::{
    buff_actions::blood_pool_ex::{
        build_blood_pool_ex_point_step, build_blood_pool_gain_ex_point_step,
    },
    context::FightContext,
    passives::collector::CollectedPassives,
    trigger::combat::TriggerEvent,
};

use super::TriggerPass;

pub struct BloodPoolSyncPass;

impl TriggerPass for BloodPoolSyncPass {
    fn run(
        &self,
        ctx: &mut FightContext<'_>,
        event: &TriggerEvent,
        _collected: &CollectedPassives,
    ) -> Vec<FightStep> {
        let mut out = Vec::new();

        let gains = [(1, event.bloodpool_gain(1)), (2, event.bloodpool_gain(2))];

        if let Some(step) = build_blood_pool_gain_ex_point_step(
            &ctx.mechanics.bloodtithe,
            ctx.fight,
            &ctx.managers.buff_mgr,
            &mut ctx.managers.entity_mgr,
            &gains,
            &event.bloodpool_gain_by_skill_team,
        ) {
            out.push(step);
        }

        if let Some(step) = build_blood_pool_ex_point_step(
            &mut ctx.mechanics.bloodtithe,
            ctx.fight,
            &ctx.managers.buff_mgr,
            &mut ctx.managers.entity_mgr,
        ) {
            out.push(step);
        }

        out
    }
}
