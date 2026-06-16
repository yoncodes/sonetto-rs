//! `SettleDotAndCostDotDuration` (skill_behavior id 60073) — round-start
//! Poison-settle on the carrier. Tuesday's `30980151` is the only fixture
//! caller today, delivered to enemies via `magic_circle 22100003.enemy_skills`.
//!
//! Per the in-game text on `30980151`: "At the start of the round, resolve 1
//! round of [Poison] effects." The carrier walks its own Poison /
//! DeadlyPoison buffs and emits a single consolidated Genesis-crit damage
//! sum equal to `Σ (poisoner.atk × permille / 1000 × stacks) × 1.39` across
//! all such buffs. The same crit-hybrid multiplier (`1.39`) the regular
//! round-end DOT settlement uses applies here so the kill thresholds line
//! up — see `mechanics/dot.rs` module doc.
//!
//! Empirical LIVE shape (battle3 r5–r9, all `30980151` emissions):
//!
//! ```text
//! actType=SKILL actId=30980151 fromId=carrier toId=carrier
//!   actEffect:
//!     effectType=131 (OriginCrit) effectNum=Σdamage targetId=carrier configEffect=60073
//!     effectType=9   (Dead)        effectNum=0      targetId=carrier  // only if carrier dies
//! ```
//!
//! ## Why no duration decrement here
//!
//! The behavior name suggests it consumes duration ("CostDotDuration"), but
//! Tuesday's array `22100003` always pairs `30980151` with the lock-duration
//! debuff `30980131` (features `"810"`, `LockPoison`). The lock pins
//! `duringTime` so the same Poison stacks keep ticking damage every round
//! the array is active — "if tick is 2 after 3 rounds it still be 2 not 0".
//!
//! For non-Tuesday-array carriers the standard `BuffMgr::on_round_end`
//! handles duration decrement. So this handler doesn't decrement at all —
//! the regular tick path is the source of truth for buff lifetime, and the
//! lock prevents that path from firing on locked carriers.
use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    mechanics::{Mechanics, dot::parse_dot_features},
    skill::{SkillExecutor, get_entity},
    utils::apply_real_hurt_fix,
};
use rand::rngs::StdRng;
use sonettobuf::Fight;

/// Crit-hybrid multiplier — same value `mechanics/dot.rs` uses on its
/// round-end Poison ticks. LIVE emits these as `et=131 OriginCrit` with
/// damage ≈ 1.39× the un-crit value; we match.
const CRIT_PERMILLE: i32 = 1390;
/// configEffect marker for 60073 emissions. Lets downstream observers
/// recognize "Poison settled by Tuesday's array" vs regular round-end
/// DOT (which has `configEffect = 0`).
const SETTLE_CONFIG_EFFECT: i32 = 60073;

// `SettleDotAndCostDotDuration` (skill_behavior id 60073) — fires on
// each enemy carrying skill 30980151 as a round-start passive (delivered
// via `magic_circle 22100003.enemy_skills`). Per the in-game text on
// 30980151: "At the start of the round, resolve 1 round of [Poison]
// effects." The carrier walks its own Poison/DeadlyPoison buffs, deals
// `caster.atk × permille / 1000` Genesis damage per stack, and
// decrements `duringTime` by `rounds` — except when the carrier also
// holds a `LockPoison(810)` buff (Tuesday's 30980131 lock-duration
// debuff also applied by the array via `enemy_buff`), in which case the
// damage still emits but `duringTime` stays pinned. LIVE encodes 60073
// as `60073#<rounds>` (30980151 slot 1 = `60073#1` = 1 round per tick).
pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    _targets: Vec<i64>,
    entity_uid: i64,
    _raw: &str,
    _count: i32,
    _beh_type: BehaviourType,
) -> Vec<Event> {
    // The carrier is the entity executing the passive — for Tuesday's
    // array, this is each living enemy reached by the circle's
    // `enemy_skills` advertisement. The skill is self-targeted so
    // `target` and `caster_uid` agree.
    let carrier = entity_uid;
    if carrier == 0 {
        return vec![];
    }

    let buffs = managers.buff_mgr.get(carrier).to_vec();
    if buffs.is_empty() {
        return vec![];
    }

    let mut total_damage: i32 = 0;
    for instance in &buffs {
        let Some((_marker_et, permille)) = parse_dot_features(instance.buff_id) else {
            continue;
        };
        let Some(poisoner) = get_entity(fight, instance.from_uid) else {
            continue;
        };
        let poisoner_atk = poisoner.attr.as_ref().and_then(|a| a.attack).unwrap_or(0);
        if poisoner_atk <= 0 {
            continue;
        }
        let base = apply_real_hurt_fix(&managers.buff_mgr, carrier, poisoner_atk * permille / 1000);
        if base <= 0 {
            continue;
        }
        let stacks = instance.layer.max(1);
        let crit_dmg = base.saturating_mul(CRIT_PERMILLE) / 1000;
        total_damage = total_damage.saturating_add(crit_dmg.saturating_mul(stacks));
    }

    if total_damage <= 0 {
        return vec![];
    }

    let mut effects = vec![Event::SerializedActEffect {
        effect: ActEffectBuilder::origin_crit(carrier, total_damage, Some(SETTLE_CONFIG_EFFECT)),
    }];

    if let Some(victim) = get_entity(fight, carrier) {
        let hp = victim.current_hp.unwrap_or(0);
        let shield = victim.shield_value.unwrap_or(0);
        let after_shield = total_damage.saturating_sub(shield);
        // Append `et=9 Dead` if the consolidated damage drops the carrier
        // below 0 HP — matches LIVE r5 step where enemy `-5` dies from a
        // single 7967 settle hit and the SKILL wrapper carries both packets.
        if after_shield > 0 && hp - after_shield <= 0 {
            effects.push(Event::SerializedActEffect {
                effect: ActEffectBuilder::dead(carrier),
            });
            effects.push(Event::SerializedActEffect {
                effect: ActEffectBuilder::remove_entity_cards(carrier, Some(1)),
            });
        }
    }

    effects
}
