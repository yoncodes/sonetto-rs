use crate::state::battle::{event::Event, manager::fight_data_mgr::Managers};

/// _100Attr: "100#<attr_id>#<amount>"
/// Writes amount into entity_mgr.attr_bonus and returns the ref for expiry cleanup.
pub fn execute(managers: &mut Managers, entity_uid: i64, params: &str) -> (Vec<Event>, Vec<(i64, i32, i32)>) {
    let mut parts = params.split('#');
    let _id = parts.next(); // skip act id
    let attr_id: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let amount: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    if attr_id == 0 || amount == 0 {
        return (vec![], vec![]);
    }
    let mut map = std::collections::HashMap::new();
    map.insert((entity_uid, attr_id), vec![amount]);
    managers.entity_mgr.merge_attr_bonus(map);
    (vec![], vec![(entity_uid, attr_id, amount)])
}
