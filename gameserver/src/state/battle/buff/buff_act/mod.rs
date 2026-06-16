pub mod buff_replace;
pub mod shield;
mod attr;
mod r#type;

use sonettobuf::Fight;
use crate::state::battle::{effect::condition::Hook, event::Event, manager::fight_data_mgr::Managers};
pub use r#type::BuffActType;

pub trait BuffActExecutor {
    const HOOKS: &'static [Hook];
    fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, params: &str, carrier_buff_id: i32) -> (Vec<Event>, Vec<(i64, i32, i32)>);
}

fn hooks_for(act_type: BuffActType) -> &'static [Hook] {
    match act_type {
        BuffActType::_501Shield => shield::ShieldAct::HOOKS,
        _ => &[Hook::EnterFight],
    }
}

#[derive(Debug, Clone)]
pub struct BuffAction {
    pub act_type: BuffActType,
    pub hooks: &'static [Hook],
    pub params: String,
}

impl BuffAction {
    pub fn new(id: i32) -> Option<Self> {
        let act_type = BuffActType::from_id(id)?;
        Some(Self { hooks: hooks_for(act_type), act_type, params: String::new() })
    }

    pub fn execute(&self, fight: &Fight, managers: &mut Managers, entity_uid: i64, carrier_buff_id: i32) -> (Vec<Event>, Vec<(i64, i32, i32)>) {
        execute(self, fight, managers, entity_uid, carrier_buff_id)
    }
}

pub fn execute(
    action: &BuffAction,
    fight: &Fight,
    managers: &mut Managers,
    entity_uid: i64,
    carrier_buff_id: i32,
) -> (Vec<Event>, Vec<(i64, i32, i32)>) {
    match action.act_type {
        BuffActType::_100Attr => attr::execute(managers, entity_uid, &action.params),
        BuffActType::_702BuffReplace => (
            buff_replace::execute(fight, managers, entity_uid, &action.params, carrier_buff_id),
            vec![],
        ),
        BuffActType::_501Shield => shield::ShieldAct::execute(fight, managers, entity_uid, &action.params, carrier_buff_id),
        other => {
            tracing::warn!("unimplemented buff act type: {:?}", other);
            (vec![], vec![])
        }
    }
}
