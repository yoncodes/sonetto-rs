mod apply;
pub mod apply_skill;
pub mod buff_act;
pub mod helper;
pub mod utils;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefreshPolicy {
    /// [`IncludeType::Stacked`] + excludeTypes: clears conflicting buffs on apply, then adds new instance.
    ReplaceOnExcludedOverlap,
    /// [`IncludeType::Unique`]: re-applying replaces the existing instance in-place.
    ReplaceOnSelfRefresh,
    /// Default: add or update the existing instance without removing others.
    #[default]
    UpdateInPlace,
}

pub use apply::{apply_buff_effects, pre_buff_effects};

// Immunity gate: when applying a buff, check if the target holds any buff whose
// `excludeTypes = "1#<type,...>"` covers the incoming buff's category. If so, reject it.
// Most immunity buffs (bt=8091, bt=8309, bt=11940091, bt=2130105) encode this via excludeTypes.
// Exception: bt=901107321 ("Cannot gain [Stats Up], [Pos Status], [Counter], or [Channel]")
// has empty excludeTypes despite its blocking description — needs special handling.

use crate::state::battle::{
    effect::condition::Hook, event::Event, manager::fight_data_mgr::Managers,
    types::buff::{ExcludeRule, IncludeType, TakeActBase, TakeStage},
};
use sonettobuf::Fight;

#[derive(Debug, Clone)]
pub struct Buff {
    pub buff_id: i32,
    pub duration: i32,
    pub stacks: i32,
    pub actions: Vec<buff_act::BuffAction>,
    pub proto_buff: sonettobuf::BuffInfo,
    pub buff_type: Option<config::skill_bufftype::SkillBufftype>,
    pub layer: i32,
    pub refresh_policy: RefreshPolicy,
    pub attr_bonus_refs: Vec<(i64, i32, i32)>,
    pub include_type: Option<IncludeType>,
    pub include_max_stacks: Option<i32>,
    pub exclude_rules: Vec<ExcludeRule>,
    pub take_stage: Option<TakeStage>,
    pub take_act: Option<TakeActBase>,
    pub shield_value: i32,
}

impl Buff {
    pub fn fire_hook(
        &mut self,
        hook: Hook,
        fight: &Fight,
        managers: &mut Managers,
        entity_uid: i64,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        for a in self.actions.iter().filter(|a| a.hooks.contains(&hook)) {
            let (evts, refs) = a.execute(fight, managers, entity_uid, self.buff_id);
            events.extend(evts);
            self.attr_bonus_refs.extend(refs);
        }
        events
    }
}
