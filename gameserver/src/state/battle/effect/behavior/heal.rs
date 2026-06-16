//! Heal action — handles the two heal-shaped skill_behavior variants:
//! `BehaviorType::Heal { rate }` (covers `Heal` and `HealCantCrit` in
//! the parser) and `BehaviorType::HealByTwoAttr { missing_percent,
//! caster_hp_percent }` (the `HealByTwoAttr` variant which scales by
//! the caster's max HP and the target's missing HP).
//!
//! Both delegate to one-shot helpers in `buff_actions::heal`. The
//! action module owns the dispatch wiring and the effect-context
//! construction; the per-formula math lives in the buff_actions
//! handler so it can also be invoked from non-skill paths (e.g. cure
//! features on buff apply).
use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    buff_actions::{EffectContext, heal as heal_handler},
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::SkillExecutor,
};
use crate::state::battle::effect::behavior::r#type::BehaviourType;

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    let mut out = Vec::new();
    for target in targets {
        let mut effect_ctx = EffectContext::new(fight, managers, mechanics, entity_uid, target);
        let effects = match beh_type {
            BehaviourType::_60232HealByTwoAttr => {
                let missing_percent: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let caster_hp_percent: i32 = raw.split('#').nth(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                heal_handler::heal_by_two_attr(&mut effect_ctx, missing_percent, caster_hp_percent)
            }
            _ => {
                let rate: i32 = raw.split('#').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                heal_handler::heal(&mut effect_ctx, rate)
            }
        };
        out.extend(effects.into_iter().map(|e| Event::SerializedActEffect { effect: e }));
    }
    out
}
