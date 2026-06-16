//! Trait + context bundle + registry every behavior action module
//! implements / consults.
//!
//! Each `BehaviorType` variant is owned by exactly one action module
//! under `behavior/`. The module exposes a unit struct (e.g.
//! `damage::Damage`, `heal::Heal`) that implements [`BehaviorAction`].
//! The dispatcher in `behavior/mod.rs::dispatch_impl` iterates
//! [`BEHAVIOR_REGISTRY`] and picks the first cluster whose `execute`
//! returns `Some(...)`.
//!
//! Adding a new behavior cluster:
//! 1. Create the module under `behavior/`.
//! 2. Add `pub(super) struct ClusterName;` and `impl BehaviorAction
//!    for ClusterName { ... }`.
//! 3. Add `&cluster::ClusterName` to `BEHAVIOR_REGISTRY` below.
//!
//! Return semantics: `Some(Ok(effects))` = handled; `None` = foreign
//! variant (let the registry continue iterating). The dispatcher
//! emits `Ok(vec![])` if no cluster claims the variant.

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::ActEffect;

use super::super::executor::SkillExecutor;
use crate::state::battle::{
    context::behavior_context::BehaviorContext, manager::fight_data_mgr::Managers,
    mechanics::Mechanics, types::behavior::BehaviorType, types::condition::ConditionType,
};

/// Bundle of mutable handles + per-target invocation data threaded
/// into every action's `execute`. Fields are `pub(super)` so siblings
/// (action modules under `behavior/`) can read them directly without
/// an accessor explosion. The dispatcher constructs this once per
/// target inside `dispatch_impl`.
#[allow(dead_code)]
pub(crate) struct ActionCtx<'a, 'ctx> {
    pub executor: &'a mut SkillExecutor,
    pub rng: &'a mut StdRng,
    pub managers: &'a mut Managers,
    pub mechanics: &'a mut Mechanics,
    pub behavior_ctx: &'a BehaviorContext<'ctx>,
    pub caster_uid: i64,
    pub target: i64,
    pub skill_id: i32,
    pub condition_id: i32,
}

/// Contract for every behavior action module.
///
/// Each cluster pattern-matches the `BehaviorType` variants it owns
/// inside `execute` and returns `None` for foreign variants. The
/// registry's iteration falls through to the next cluster on `None`.
pub(super) trait BehaviorAction {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>>;
}

/// Ordered registry of every behavior action cluster the dispatcher
/// consults. First cluster to return `Some(...)` wins.
///
/// Order matches the Phase 1+2+3 dispatch sequence so the same
/// fall-through behavior holds when two clusters could claim
/// overlapping variants.
pub(super) const BEHAVIOR_REGISTRY: &[&dyn BehaviorAction] = &[];
