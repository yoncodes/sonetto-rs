pub(crate) mod action;

use self::action::{ActionCtx, BEHAVIOR_REGISTRY};

pub mod parser;

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::ActEffect;

use super::cache::resolve_skill_effect_id;
use super::executor::SkillExecutor;
use crate::state::battle::{
    context::behavior_context::BehaviorContext,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::cache::SKILL_CACHE,
    skill::condition::parser::parse_condition,
    types::{behavior::BehaviorType, condition::ConditionType},
};

pub struct BehaviorExec<'a, 'ctx> {
    executor: &'a mut SkillExecutor,
    rng: &'a mut StdRng,
    managers: &'a mut Managers,
    mechanics: &'a mut Mechanics,
    behavior_ctx: &'a BehaviorContext<'ctx>,
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
    condition_id: i32,
}

impl<'a, 'ctx> BehaviorExec<'a, 'ctx> {
    pub fn new(
        executor: &'a mut SkillExecutor,
        rng: &'a mut StdRng,
        managers: &'a mut Managers,
        mechanics: &'a mut Mechanics,
        behavior_ctx: &'a BehaviorContext<'ctx>,
    ) -> Self {
        Self {
            executor,
            rng,
            managers,
            mechanics,
            behavior_ctx,
            caster_uid: behavior_ctx.caster_uid,
            target_uid: behavior_ctx.target_uid,
            skill_id: behavior_ctx.skill_id,
            condition_id: 0,
        }
    }

    pub fn caster(mut self, caster_uid: i64) -> Self {
        self.caster_uid = caster_uid;
        self
    }

    pub fn for_target(mut self, target_uid: i64) -> Self {
        self.target_uid = target_uid;
        self
    }

    pub fn skill(mut self, skill_id: i32) -> Self {
        self.skill_id = skill_id;
        self
    }

    pub fn condition_uid(mut self, condition_id: i32) -> Self {
        self.condition_id = condition_id;
        self
    }

    pub fn run(self, behavior: &BehaviorType, condition: &ConditionType) -> Result<Vec<ActEffect>> {
        dispatch_impl(
            self.executor,
            self.rng,
            self.managers,
            self.mechanics,
            self.behavior_ctx,
            self.caster_uid,
            self.target_uid,
            behavior,
            self.skill_id,
            self.condition_id,
            condition,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn execute_behavior(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    behavior_ctx: &BehaviorContext<'_>,
    behavior: &BehaviorType,
    condition_id: i32,
    condition: &ConditionType,
) -> Result<Vec<ActEffect>> {
    let caster_uid = behavior_ctx.caster_uid;
    let skill_id = behavior_ctx.skill_id;
    tracing::debug!(
        "execute_behavior slot={} skill={} caster={} target={}",
        behavior_ctx.slot,
        skill_id,
        caster_uid,
        behavior_ctx.target_uid
    );
    let self_targeted = matches!(
        behavior,
        BehaviorType::Bloodlust { .. }
            | BehaviorType::AddExPoint { .. }
            | BehaviorType::DirectUseBigSkill
            | BehaviorType::SettleDotAndCostDotDuration { .. }
    );

    let targets = behavior_ctx.resolve_targets(self_targeted, behavior);
    let mut effects = Vec::new();
    for target in targets {
        effects.extend(
            BehaviorExec::new(executor, rng, managers, mechanics, behavior_ctx)
                .caster(caster_uid)
                .for_target(target)
                .skill(skill_id)
                .condition_uid(condition_id)
                .run(behavior, condition)?,
        );
    }
    Ok(effects)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_impl(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    behavior_ctx: &BehaviorContext<'_>,
    caster_uid: i64,
    target: i64,
    behavior: &BehaviorType,
    skill_id: i32,
    condition_id: i32,
    condition: &ConditionType,
) -> Result<Vec<ActEffect>> {
    let mut action_ctx = ActionCtx {
        executor,
        rng,
        managers,
        mechanics,
        behavior_ctx,
        caster_uid,
        target,
        skill_id,
        condition_id,
    };
    for cluster in BEHAVIOR_REGISTRY {
        if let Some(result) = cluster.execute(behavior, &mut action_ctx, condition) {
            return result;
        }
    }
    Ok(vec![])
}
