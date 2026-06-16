use sonettobuf::Fight;
use crate::state::battle::{context::hook_call, event::Event, manager::fight_data_mgr::Managers};

pub fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, params: &str, carrier_buff_id: i32) -> Vec<Event> {
    // params format: "act_id#threshold#target_buff_id"
    let mut parts = params.split('#');
    let _act_id = parts.next();
    let threshold: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let target_buff_id: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    if target_buff_id == 0 {
        return vec![];
    }

    let stacks = managers.buff_mgr.active_buff
        .get(&entity_uid)
        .and_then(|bs| bs.iter().find(|b| b.buff_id == carrier_buff_id))
        .map(|b| b.stacks)
        .unwrap_or(0);

    if stacks < threshold {
        return vec![];
    }

    let has_target = managers.buff_mgr.active_buff
        .get(&entity_uid)
        .is_some_and(|bs| bs.iter().any(|b| b.buff_id == target_buff_id));

    let mut events = vec![Event::RemoveBuff { target_uid: entity_uid, buff_id: carrier_buff_id }];

    if has_target {
        managers.buff_mgr.extend_buff_duration(entity_uid, target_buff_id, 1);
    } else {
        managers.buff_mgr.add_buff(entity_uid, target_buff_id);
        events.extend(hook_call::on_buff_add(managers, fight, entity_uid));
        managers.buff_mgr.finalize_buff_add(entity_uid, target_buff_id);
    }

    events
}
