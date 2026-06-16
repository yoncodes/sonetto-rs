use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{SkillExecutor, buff},
};
use rand::rngs::StdRng;
use sonettobuf::Fight;

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    targets: Vec<i64>,
    entity_uid: i64,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
    skill_id: i32,
) -> Vec<Event> {
    targets
        .into_iter()
        .flat_map(|target| {
            let effects = match beh_type {
                // `Disperse2` (skill_behavior id 30009) is a single-buff drop —
                // e.g. Sentinel `31260131 slot4 = '30009#31260121'`. The
                // type-name wildcard below (`starts_with("Disperse")`) catches
                // it as the argless `Disperse` (drop every buff), discarding
                // the buff_id. Route this specific id to the existing
                // `DisperseForce` runtime to preserve the targeted-buff
                // semantic. Other Disperse-family ids (e.g. 30003 / 30004 /
                // 30008 / 30016 / 30017 / 90002) keep the legacy mapping until
                // we have evidence each carries a buff_id arg LIVE-side; the
                // wildcard route already widens enough to be wrong if their
                // semantics also differ — addressed one id at a time.
                BehaviourType::_30003Disperse1
                | BehaviourType::_30004Disperse2
                | BehaviourType::_30008Disperse1
                | BehaviourType::_30009Disperse2
                | BehaviourType::_30016Disperse3
                | BehaviourType::_30017Disperse4
                | BehaviourType::_90002Disperse2 => buff::disperse(fight, managers, target),
                BehaviourType::_60010DisperseForce2 | BehaviourType::_60011DisperseForce1 => {
                    let buff_id: i32 = raw
                        .split('#')
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    buff::disperse_force(fight, managers, target, buff_id)
                }
                BehaviourType::_20003Purify1
                | BehaviourType::_20004Purify2
                | BehaviourType::_20020PurifyX => buff::purify(fight, managers, target),
                BehaviourType::_50014ConsumeBuffByTypeId
                | BehaviourType::_50016ConsumeBuffByTypeId2 => {
                    let type_id: i32 = raw
                        .split('#')
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let count: i32 = raw
                        .split('#')
                        .nth(2)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    buff::consume_by_type(fight, managers, target, type_id, skill_id, count)
                }
                BehaviourType::_60176ReplaceBuff2 => {
                    let parts: Vec<&str> = raw.split('#').collect();
                    let source_buff_ids: Vec<i32> = parts
                        .get(1)
                        .map(|p| p.split(',').filter_map(|v| v.parse().ok()).collect())
                        .unwrap_or_default();
                    let replacement_buff_id: i32 =
                        parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                    let duration: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                    let count: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(1);
                    buff::replace_buff2(
                        fight,
                        managers,
                        entity_uid,
                        target,
                        &source_buff_ids,
                        replacement_buff_id,
                        duration,
                        count,
                    )
                }
                _ => vec![],
            };
            effects
                .into_iter()
                .map(|e| Event::SerializedActEffect { effect: e })
        })
        .collect()
}
