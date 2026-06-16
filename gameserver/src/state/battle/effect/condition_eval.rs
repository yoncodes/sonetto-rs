use sonettobuf::Fight;
use std::collections::HashSet;
use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr},
    mechanics::bloodtithe::BloodtitheState,
    skill::classification_kind::SkillEmitContext,
};

#[derive(Clone, Copy)]
pub struct ConditionEval<'a> {
    pub fight: &'a Fight,
    pub buff_mgr: &'a BuffMgr,
    pub entity_mgr: &'a EntityMgr,
    pub bloodtithe: &'a BloodtitheState,
    pub caster_uid: i64,
    pub target_uid: i64,
    pub condition_target: i32,
    pub has_trigger_state: bool,
    pub active_card_cast_uids: Option<&'a HashSet<i64>>,
    pub lost_buff_id: Option<i32>,
    pub emit_context: Option<SkillEmitContext>,
}
