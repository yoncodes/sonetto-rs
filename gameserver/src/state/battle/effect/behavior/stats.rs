//! Stats action — handler for the small "emit a single stat-change
//! ActEffect" behavior variants that don't fit a more specific action
//! module:
//!
//! * `Bloodlust { amount }` — `effectType = Bloodlust`
//! * `ChangePower { amount }` — `effectType = Powerchange` with
//!   `config_effect = 1`
//! * `AverageLife` — `effectType = Averagelife`
//!
//! ExPoint-shaped variants (`AddExPoint`, `AddExPointWithMax`,
//! `ConsumeExPointAddAttr`) live in `ex_point.rs` because they all
//! key off the per-entity ExPoint counter (Moxie / Faith). Skill-rate
//! buffs (`SkillRateUp*`) live inline in the dispatcher for now.
use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::SkillExecutor,
};
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, Fight};

pub fn bloodlust(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![ActEffectBuilder::bloodlust(target, amount)]
}

pub fn change_power(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![ActEffectBuilder::power_change(Some(target), amount, Some(1))]
}

pub fn average_life(target: i64) -> Vec<ActEffect> {
    vec![ActEffectBuilder::average_life(target)]
}

pub fn execute(
    _fight: &Fight,
    _managers: &mut Managers,
    _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    targets: Vec<i64>,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
) -> Vec<Event> {
    let amount: i32 = raw
        .split('#')
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    targets
        .into_iter()
        .flat_map(|target| {
            let effects = match beh_type {
                BehaviourType::_20010Bloodlust => bloodlust(target, amount),
                BehaviourType::_20011AverageLife => average_life(target),
                BehaviourType::_50017ChangePower | BehaviourType::_50037ChangePower => {
                    change_power(target, amount)
                }
                _ => vec![],
            };
            effects
                .into_iter()
                .map(|e| Event::SerializedActEffect { effect: e })
        })
        .collect()
}
