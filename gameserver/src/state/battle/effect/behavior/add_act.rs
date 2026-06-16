use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;

pub fn execute(managers: &mut Managers, targets: Vec<i64>, raw: &str, count: i32) -> Vec<Event> {
    let amount: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let total_amount = amount * count;
    targets.into_iter().map(|uid| managers.entity_mgr.add_action_point(uid, total_amount)).collect()
}