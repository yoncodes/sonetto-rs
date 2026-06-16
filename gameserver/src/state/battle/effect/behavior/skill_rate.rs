//! SkillRate action — handler for the three behavior variants that
//! register a per-skill rate bonus on the executor (the bonus is
//! consumed later when the host skill computes damage):
//!
//! * `SkillRateUp { rate }` — unconditional bonus of `rate` permille
//!   from caster to current target.
//! * `SkillRateUpBySelfBuffType { buff_type_id, rate }` — bonus of
//!   `rate × stacks` where `stacks` is the count of buffs on the
//!   caster matching `buff_type_id`. Returns empty if the caster
//!   has no matching buffs or `rate == 0`.
//! * `SkillRateUpByBuffType { rate, buff_types }` — bonus of `rate`
//!   if the target carries any buff matching one of the listed
//!   types. Returns empty if no match or `rate == 0`.
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
    executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    caster_uid: i64,
    targets: Vec<i64>,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
) -> Vec<Event> {
    let parts: Vec<&str> = raw.split('#').collect();
    let p1: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let p2: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);

    match beh_type {
        // `CritRateAlter2` (skill_behavior id 60228) bumps the caster's
        // Crit Rate (Attr::Cri = 201). LIVE encodes it as `60228#<permille>`
        // — e.g. Sentinel `31260181 slot2 = '60228#800'` (+80% Cri while
        // her Hour of Repentance buff 31260151 is up). Route to the
        // existing `AttrFix` runtime: the Cri attr is one of the
        // attribute slots `executor.add_attr_bonus` already updates.
        // Note: id 100023 is also tagged `CritRateAlter2` in the data
        // bundle but is not in current fixtures — leaving it
        // unaliased until we see a LIVE use.
        BehaviourType::_10001SkillRateUp
        | BehaviourType::_10002SkillRateUp1
        | BehaviourType::_10003SkillRateUp2
        | BehaviourType::_40015Rouge2MusicBlueBallSkillRateUp
        | BehaviourType::_60028ConsumePowerSkillRateUp => {
            // raw: id#rate
            for &t in &targets {
                executor.add_skill_rate_bonus(caster_uid, t, p1);
            }
        }
        BehaviourType::_10009SkillRateUpExPoint => {
            // raw: id#buff_type_id#rate — bonus = rate * stacks(buff_type_id on caster)
            let buff_type_id = p1;
            let rate = p2;
            if rate != 0 {
                let stacks = buff::sum_stacks_by_type(fight, managers, caster_uid, buff_type_id);
                if stacks > 0 {
                    for &t in &targets {
                        executor.add_skill_rate_bonus(caster_uid, t, rate.saturating_mul(stacks));
                    }
                }
            }
        }
        BehaviourType::_10012SkillRateUpBuffType => {
            // raw: id#rate#?#buff_type1#buff_type2...
            let rate = p1;
            if rate != 0 {
                let buff_types: Vec<i32> = parts
                    .iter()
                    .skip(3)
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if !buff_types.is_empty() {
                    for &t in &targets {
                        if buff::has_any_type(fight, managers, t, &buff_types) {
                            executor.add_skill_rate_bonus(caster_uid, t, rate);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    vec![]
}
