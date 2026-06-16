// Act 791 — Shield
//
// Computes a shield amount from a stat and applies it to the carrier on BuffAdd.
// params: `791#<use_missing>#<attr_id>#<permille>#...`
//   use_missing: 1 = base is (max_hp - current_hp), 0 = use attr directly
//   attr_id:     100=current_hp, 101=max_hp, 102=atk, 103=def
//   permille:    multiplier in 1/1000 units (e.g. 200 = 20%)
//
// Real shield buffs (e.g. buff 30940121, typeId=5001) use excludeTypes "1#7" so applying
// one removes all existing type-7 buffs first. The two type-7 bufftypes with empty
// excludeTypes (72000005, 30940121) are either internal/unnamed or dead — no real buff
// uses bufftype 30940121, and buff 72000005 is an unnamed internal buff.
//
// `Buff.shield_value` is the live remaining shield amount and is the single source of
// truth. It is set here via `buff_mgr.set_shield_value` and decremented during damage
// absorption. `entity.shield_value` mirrors the aggregate for fast lookup during combat.
use sonettobuf::Fight;
use crate::state::battle::{
    effect::condition::Hook,
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    utils::find_entity,
};
use super::BuffActExecutor;

pub struct ShieldAct;

impl BuffActExecutor for ShieldAct {
    const HOOKS: &'static [Hook] = &[Hook::BuffAdd];

    fn execute(fight: &Fight, managers: &mut Managers, entity_uid: i64, params: &str, carrier_buff_id: i32) -> (Vec<Event>, Vec<(i64, i32, i32)>) {
        let mut parts = params.split('#');
        let _id = parts.next();
        let use_missing: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let attr_id: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        let permille: i32 = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        if permille == 0 { return (vec![], vec![]); }

        let entity = find_entity(fight, entity_uid);
        let base = if use_missing != 0 {
            let max_hp = entity.and_then(|e| e.attr.as_ref()).and_then(|a| a.hp).unwrap_or(0);
            let cur_hp = managers.entity_mgr.current_hp.get(&entity_uid).copied().unwrap_or(0);
            (max_hp - cur_hp).max(0)
        } else {
            let attr = entity.and_then(|e| e.attr.as_ref());
            match attr_id {
                100 => managers.entity_mgr.current_hp.get(&entity_uid).copied().unwrap_or(0),
                101 => attr.and_then(|a| a.hp).unwrap_or(0),
                102 => attr.and_then(|a| a.attack).unwrap_or(0),
                103 => attr.and_then(|a| a.defense).unwrap_or(0),
                _ => 0,
            }
        };

        let amount = base * permille / 1000;
        if amount <= 0 { return (vec![], vec![]); }

        managers.buff_mgr.set_shield_value(entity_uid, carrier_buff_id, amount);

        let effect = ActEffectBuilder::shield(entity_uid, amount);
        (vec![Event::SerializedActEffect { effect }], vec![])
    }
}
