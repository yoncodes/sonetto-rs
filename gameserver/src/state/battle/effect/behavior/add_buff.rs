use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::{Mechanics, bloodtithe::bloodtithe_add_to_pool},
    skill::{SkillExecutor, buff},
};
use crate::state::battle::effect::behavior::r#type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, count: i32, beh_type: BehaviourType,
    skill_id: i32,
) -> Vec<Event> {
    targets.into_iter().flat_map(|target| {
        let effects = match beh_type {
            BehaviourType::_1AddBuff | BehaviourType::_20005AddBuffRound | BehaviourType::_20017AddBuffRound2 => {
                let buff_id: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                buff::apply(
                    buff::BuffApplySpec::new(buff_id)
                        .caster(entity_uid).target(target).count(count)
                        .bloodpool(mechanics.bloodtithe.has_bloodpool())
                        .skill(skill_id),
                    executor, fight, managers, mechanics,
                )
            }
            BehaviourType::_60210ConsumeBloodAddBuff | BehaviourType::_60211ConsumeBloodAddBuff2 => {
                let parts: Vec<&str> = raw.split('#').collect();
                let consume: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let buff_id: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                let cnt: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                let current = mechanics.bloodtithe.get_value(1);
                if current < consume { return vec![]; }
                executor.side_effects.push(bloodtithe_add_to_pool(target, -consume));
                mechanics.bloodtithe.add_value(1, -consume);
                buff::apply(
                    buff::BuffApplySpec::new(buff_id)
                        .caster(entity_uid).target(target).count(cnt)
                        .bloodpool(mechanics.bloodtithe.has_bloodpool())
                        .skill(skill_id),
                    executor, fight, managers, mechanics,
                )
            }
            _ => vec![],
        };
        effects.into_iter().map(|e| Event::SerializedActEffect { effect: e }).collect::<Vec<_>>()
    }).collect()
}
