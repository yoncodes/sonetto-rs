use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use sonettobuf::Fight;
use std::collections::HashMap;

/// Compute the attr-bonus contribution this behaviour would add for `entity_uid`.
/// Returns a map keyed by `(uid, attr_id) -> amount`. The merge into
/// `entity_mgr.attr_bonus` is the caller's responsibility (Vec<i32> aggregation
/// happens at that layer).
///
/// Supports raw strings from the `_10004AttrFix`, `_10011AttrFixBuff`, and
/// `_60033AttrFixByLoseHp` BehaviourType variants.
pub fn calculate_bonus(
    fight: &Fight,
    _managers: &Managers,
    entity_uid: i64,
    raw: &str,
    count: i32,
) -> HashMap<(i64, i32), i32> {
    let mut parts = raw.split('#');
    let id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = HashMap::new();
    match id {
        // _10004AttrFix         "10004#<attr_id>#<amount>"
        // _10011AttrFixBuff     "10011#<attr_id>#<amount>#<buff_id>" (buff_id ignored at this layer)
        10004 | 10011 => {
            let attr_id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let amount: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let scaled = amount.saturating_mul(count.max(1));
            if attr_id != 0 && scaled != 0 {
                out.insert((entity_uid, attr_id), scaled);
            }
        }
        // _60033AttrFixByLoseHp "60033#<step_permille>#<attr_id>#<bonus_per_stack>#<max_stacks>"
        // `AttrFixByLoseHp` (skill_behavior id 60033) is encoded as
        // `60033#<step_permille>#<attr_id>#<bonus_per_stack>#<max_stacks>`.
        // Semmelweis Insight III 308801821 slot 6 carries
        // `60033#100#205#75#8` — i.e. for each 10% of MaxHP missing
        // on the caster, grant +7.5% AddDmg (attr 205), capped at 8
        // stacks (60% total). The `AttrFix` wildcard below would catch
        // this by name and only read the first two args, so we route
        // by id before the wildcard.
        60033 => {
            let step_permille: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let attr_id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let bonus_per_stack: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let max_stacks: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            if step_permille <= 0 || bonus_per_stack <= 0 || max_stacks <= 0 || attr_id == 0 {
                return out;
            }
            let entity = fight
                .attacker
                .iter()
                .chain(fight.defender.iter())
                .flat_map(|s| s.entitys.iter().chain(s.sub_entitys.iter()))
                .find(|e| e.uid == Some(entity_uid));
            let Some(entity) = entity else {
                return out;
            };
            let max_hp = entity.attr.as_ref().and_then(|a| a.hp).unwrap_or(0);
            if max_hp <= 0 {
                return out;
            }
            let cur_hp = entity.current_hp.unwrap_or(0);
            let missing = (max_hp - cur_hp).max(0) as i64;
            let missing_permille = (missing * 1000 / max_hp as i64) as i32;
            let stacks = (missing_permille / step_permille).min(max_stacks);
            if stacks <= 0 {
                return out;
            }
            let bonus = stacks.saturating_mul(bonus_per_stack);
            out.insert((entity_uid, attr_id), bonus);
        }
        _ => {}
    }
    out
}

/// Side-effecting form: compute then merge into `managers.entity_mgr`. Returns
/// no events (the parallel "calculate_bonus" pass is what other code observes;
/// `execute` exists for the rare case where a behaviour fires outside of an
/// eval-hook attr-fix pass and needs to persist immediately).
pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    entity_uid: i64,
    raw: &str,
    count: i32,
) -> Vec<Event> {
    let map = calculate_bonus(fight, managers, entity_uid, raw, count);
    let mut nested: HashMap<(i64, i32), Vec<i32>> = HashMap::new();
    for (k, v) in map {
        nested.entry(k).or_default().push(v);
    }
    managers.entity_mgr.merge_attr_bonus(nested);
    Vec::new()
}
