use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{SkillExecutor, cache::resolve_skill_effect_id},
};
use crate::state::battle::effect::behavior::r#type::BehaviourType;

pub fn execute(
    _fight: &Fight, managers: &mut Managers, _mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, count: i32, beh_type: BehaviourType,
    skill_id: i32,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_20002AddExPoint => {
            let delta: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            let total_delta = delta * count;
            targets.into_iter().map(|target| {
                managers.entity_mgr.add_ex_point(target, total_delta);
                Event::ExPointChange { target, delta: total_delta, emit_step: true }
            }).collect()
        }
        BehaviourType::_60174ConsumeExPointAddAttr => {
            let parts: Vec<&str> = raw.split('#').collect();
            let min_consume: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
            let max_consume: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
            let consumed = managers.entity_mgr.get_recent_decr_ex_point(entity_uid).max(0);
            let usable = consumed.clamp(min_consume, max_consume);
            if usable <= 0 { return vec![]; }
            let cfg = config::configs::get();
            let skill_effect_id = resolve_skill_effect_id(skill_id);
            let mut rate_per_point = 0;
            if let Some(skill_row) = cfg.skill_effect.iter().find(|s| s.id == skill_effect_id) {
                for raw_b in [
                    &skill_row.behavior1, &skill_row.behavior2, &skill_row.behavior3,
                    &skill_row.behavior4, &skill_row.behavior5, &skill_row.behavior6,
                    &skill_row.behavior7, &skill_row.behavior8, &skill_row.behavior9,
                    &skill_row.behavior10,
                ] {
                    if raw_b.starts_with("60174#") {
                        rate_per_point = raw_b.split('#').nth(2)
                            .and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
                        if rate_per_point != 0 { break; }
                    }
                }
            }
            if rate_per_point == 0 { return vec![]; }
            executor.add_skill_rate_bonus(entity_uid, entity_uid, rate_per_point.saturating_mul(usable));
            vec![]
        }
        _ => vec![],
    }
}
