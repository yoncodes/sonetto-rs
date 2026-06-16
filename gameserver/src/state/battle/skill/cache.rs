use config::configs;
use once_cell::sync::Lazy;
use std::collections::HashMap;

use super::super::{BehaviorType, ConditionType};
use super::condition::parser::parse_condition;

#[derive(Debug, Clone)]
pub struct ResolvedBehavior {
    pub condition_id: i32,
    pub condition: ConditionType,
    pub negated: bool,
    pub condition_target: i32,
    pub round_limit: i32,
    pub behavior: BehaviorType,
    pub behavior_target: i32,
    pub logic_target: i32,
}

pub static SKILL_CACHE: Lazy<HashMap<i32, Vec<ResolvedBehavior>>> = Lazy::new(build_cache);

pub fn init_skill_cache() {
    let n = SKILL_CACHE.len();
    tracing::info!("SkillCache initialized: {} skill entries", n);
}

pub fn resolve_skill_effect_id(skill_id: i32) -> i32 {
    let game = configs::get();
    if game.skill_effect.get(skill_id).is_some() {
        return skill_id;
    }
    game.skill
        .get(skill_id)
        .map(|s| s.skill_effect)
        .unwrap_or(skill_id)
}

fn parse_logic_target(raw: &str) -> i32 {
    raw.parse().unwrap_or(0)
}

fn build_cache() -> HashMap<i32, Vec<ResolvedBehavior>> {
    let cfg = configs::get();
    let mut cache = HashMap::new();

    for skill in cfg.skill_effect.iter() {
        let rows = [
            (
                &skill.condition1,
                &skill.condition_target1,
                skill.round_limit1,
                &skill.behavior1,
                &skill.behavior_target1,
            ),
            (
                &skill.condition2,
                &skill.condition_target2,
                skill.round_limit2,
                &skill.behavior2,
                &skill.behavior_target2,
            ),
            (
                &skill.condition3,
                &skill.condition_target3,
                skill.round_limit3,
                &skill.behavior3,
                &skill.behavior_target3,
            ),
            (
                &skill.condition4,
                &skill.condition_target4,
                skill.round_limit4,
                &skill.behavior4,
                &skill.behavior_target4,
            ),
            (
                &skill.condition5,
                &skill.condition_target5,
                skill.round_limit5,
                &skill.behavior5,
                &skill.behavior_target5,
            ),
            (
                &skill.condition6,
                &skill.condition_target6,
                skill.round_limit6,
                &skill.behavior6,
                &skill.behavior_target6,
            ),
            (
                &skill.condition7,
                &skill.condition_target7,
                skill.round_limit7,
                &skill.behavior7,
                &skill.behavior_target7,
            ),
            (
                &skill.condition8,
                &skill.condition_target8,
                skill.round_limit8,
                &skill.behavior8,
                &skill.behavior_target8,
            ),
            (
                &skill.condition9,
                &skill.condition_target9,
                skill.round_limit9,
                &skill.behavior9,
                &skill.behavior_target9,
            ),
            (
                &skill.condition10,
                &skill.condition_target10,
                skill.round_limit10,
                &skill.behavior10,
                &skill.behavior_target10,
            ),
        ];

        let behaviors: Vec<ResolvedBehavior> = rows
            .into_iter()
            .filter(|(_, _, _, b, _)| !b.is_empty())
            .map(|(c, ct, round_limit, b, bt)| {
                let stripped = c.trim_start_matches('!').trim_start_matches('！');
                let condition_id: i32 = stripped
                    .split('#')
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let (condition, negated) = parse_condition(c);
                ResolvedBehavior {
                    condition_id,
                    condition,
                    negated,
                    condition_target: parse_logic_target(ct),
                    round_limit,
                    behavior: BehaviorType::Unknown { raw: b.to_string() },
                    behavior_target: parse_logic_target(bt),
                    logic_target: skill.logic_target.parse().unwrap_or(0),
                }
            })
            .collect();

        if !behaviors.is_empty() {
            cache.insert(skill.id, behaviors);
        }
    }

    cache
}
