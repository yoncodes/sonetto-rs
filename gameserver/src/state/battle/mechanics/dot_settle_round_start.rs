//! Round-start defender Poison settle for arrays that pin Poison
//! at round-start.
//!
//! Some heroes (Tuesday) deploy magic-circle arrays whose
//! `enemy_skills` advertise a c101+`60073` round-start Poison settle
//! to each enemy of the array owner. LIVE delivers this AHEAD of
//! every other round phase — before any pre-player attacker sweep
//! and well before any boss `Purify` skill (e.g. `114300811`) can
//! dispel the locked Poison on the carrier.
//!
//! ## Why a mechanic, not a skill dispatch
//!
//! The behavior path (`skill/behavior/dot_settle.rs`) runs through
//! `execute_passive_skill`, which is bound to round_mgr's passive
//! sweep ordering. Defender passive sweeps run AFTER
//! `phase::player_actions::run` (see `phase/non_terminal_round.rs`
//! `DefenderBootstrap` at line 98-109), so by the time they fire in
//! the round Tuesday creates the array, the array is already
//! visible — opposite of LIVE behavior, which reads the round-start
//! snapshot. Inlining the settle as a mechanic between
//! `round_open` and `phase::player_actions::run` reads the fight
//! state at true round-start: r4 (round of array creation) sees no
//! array → no settle; r5+ sees the array → settle fires before any
//! Purify can clear the Poison. See `_session_4_42_*` for the
//! DefenderBootstrap mis-placement that motivated this split.
//!
//! ## Carrier discovery (config-driven)
//!
//! 1. If `ctx.fight.magic_circle` is active, read its config's
//!    `enemy_skills` field.
//! 2. For each entry, accept it as a settle skill iff the resolved
//!    `skill_effect` has a c101 round-start condition AND a `60073`
//!    `SettleDotAndCostDotDuration` behavior in any slot.
//! 3. For each carrier (alive entity on the side OPPOSITE the
//!    array owner), if the carrier holds at least one
//!    Poison/DeadlyPoison buff, emit one settle wrapper.
//!
//! No coupling to specific skill ids; adding a future array carrier
//! with the same shape needs no engine work.
//!
//! ## Emission shape
//!
//! Mirrors `skill/behavior/dot_settle.rs::DotSettle::execute` —
//! same Σ formula, same `60073` configEffect marker, same crit
//! hybrid 1.39× multiplier shared with `mechanics/dot.rs`:
//!
//! ```text
//! actType=Effect actId=0 (outer container)
//!   actEffect:
//!     actType=Skill actId=<settle_skill_id> fromId=carrier toId=carrier
//!       actEffect:
//!         et=131 OriginCrit num=Σdamage ti=carrier configEffect=60073
//!         et=9   Dead       num=0       ti=carrier   (only if settle kills)
//! ```

use sonettobuf::{ActEffect, FightStep};

use crate::state::battle::{
    context::FightContext,
    fight_step::{ActEffectBuilder, make_skill_step, wrap_step},
    mechanics::dot::parse_dot_features,
    round::step_shape::build_effect_step,
    skill::{
        cache::resolve_skill_effect_id, condition::scope::skill_is_round_start_only,
        targets::get_entity,
    },
    utils::apply_real_hurt_fix,
};

/// Crit-hybrid multiplier shared with `mechanics/dot.rs` round-end
/// settlement. See that module's doc for the empirical basis.
const CRIT_PERMILLE: i32 = 1390;

/// Marker that downstream observers use to recognize "Poison settled
/// by a round-start array" emission shape — same value the
/// `behavior/dot_settle.rs` path stamps on its `et=131` inner.
const SETTLE_CONFIG_EFFECT: i32 = 60073;

/// `buff_act` row id whose `type` is `SettleDotAndCostDotDuration`.
/// Skills whose behavior head equals this id advertise a round-start
/// Poison settle.
const SETTLE_BEHAVIOR_ID: i32 = 60073;

/// Build the round-start defender Poison settle steps. Returns an
/// empty vec when no active array advertises a round-start settle or
/// no hostile carrier holds a Poison-family buff.
pub fn build_round_start_dot_settle_steps(ctx: &FightContext<'_>) -> Vec<FightStep> {
    let Some(circle) = ctx.fight.magic_circle.as_ref() else {
        return Vec::new();
    };
    let Some(circle_id) = circle.magic_circle_id else {
        return Vec::new();
    };
    if circle.round.unwrap_or(0) == 0 {
        return Vec::new();
    }
    let owner_uid = circle.create_uid.unwrap_or(0);
    if owner_uid == 0 {
        return Vec::new();
    }
    let cfg = config::configs::get();
    let Some(circle_cfg) = cfg.magic_circle.get(circle_id) else {
        return Vec::new();
    };

    let mut settle_skill_ids: Vec<i32> = Vec::new();
    for piece in circle_cfg
        .enemy_skills
        .split(['|', ',', ';', '#'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let Ok(skill_id) = piece.parse::<i32>() else {
            continue;
        };
        if skill_id <= 0 {
            continue;
        }
        if !is_round_start_settle_skill(skill_id) {
            continue;
        }
        if !settle_skill_ids.contains(&skill_id) {
            settle_skill_ids.push(skill_id);
        }
    }
    if settle_skill_ids.is_empty() {
        return Vec::new();
    }

    let mut wrappers: Vec<ActEffect> = Vec::new();
    for carrier_uid in iter_hostile_alive_uids(ctx.fight, owner_uid) {
        let total_damage = compute_total_settle_damage(ctx, carrier_uid);
        if total_damage <= 0 {
            continue;
        }
        for &settle_skill_id in &settle_skill_ids {
            let mut effects = vec![ActEffectBuilder::origin_crit(
                carrier_uid,
                total_damage,
                Some(SETTLE_CONFIG_EFFECT),
            )];
            if entity_will_die_from(ctx, carrier_uid, total_damage) {
                effects.push(ActEffectBuilder::dead(carrier_uid));
                effects.push(ActEffectBuilder::remove_entity_cards(carrier_uid, Some(1)));
            }
            let inner = make_skill_step(carrier_uid, carrier_uid, settle_skill_id, 0, effects);
            wrappers.push(wrap_step(inner));
        }
    }

    if wrappers.is_empty() {
        Vec::new()
    } else {
        vec![build_effect_step(wrappers)]
    }
}

/// True when `skill_id` is a round-start Poison-settle skill: every
/// non-empty condition slot is round-start scope (c100/101/102/104)
/// AND at least one behavior slot's leading id is `60073`
/// `SettleDotAndCostDotDuration`.
fn is_round_start_settle_skill(skill_id: i32) -> bool {
    if !skill_is_round_start_only(skill_id) {
        return false;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = cfg.skill_effect.get(effect_id) else {
        return false;
    };
    let slots = [
        &skill.behavior1,
        &skill.behavior2,
        &skill.behavior3,
        &skill.behavior4,
        &skill.behavior5,
        &skill.behavior6,
        &skill.behavior7,
        &skill.behavior8,
        &skill.behavior9,
        &skill.behavior10,
        &skill.behavior11,
        &skill.behavior12,
        &skill.behavior13,
        &skill.behavior14,
        &skill.behavior15,
        &skill.behavior16,
        &skill.behavior17,
        &skill.behavior18,
        &skill.behavior19,
        &skill.behavior20,
    ];
    for raw in slots {
        let Some(head) = raw.split('#').next() else {
            continue;
        };
        let Ok(act_id) = head.trim().parse::<i32>() else {
            continue;
        };
        if act_id == SETTLE_BEHAVIOR_ID {
            return true;
        }
    }
    false
}

/// Σ (caster.atk × permille / 1000 × stacks) × 1.39 across every
/// Poison-family buff on `carrier_uid`. Mirrors the behavior path.
fn compute_total_settle_damage(ctx: &FightContext<'_>, carrier_uid: i64) -> i32 {
    let buffs = ctx.managers.buff_mgr.get(carrier_uid).to_vec();
    if buffs.is_empty() {
        return 0;
    }
    let mut total: i32 = 0;
    for instance in &buffs {
        let Some((_marker_et, permille)) = parse_dot_features(instance.buff_id) else {
            continue;
        };
        let Some(caster) = get_entity(ctx.fight, instance.from_uid) else {
            continue;
        };
        let caster_atk = caster.attr.as_ref().and_then(|a| a.attack).unwrap_or(0);
        if caster_atk <= 0 {
            continue;
        }
        let base = apply_real_hurt_fix(
            &ctx.managers.buff_mgr,
            carrier_uid,
            caster_atk * permille / 1000,
        );
        if base <= 0 {
            continue;
        }
        let stacks = instance.layer.max(1);
        let crit_dmg = base.saturating_mul(CRIT_PERMILLE) / 1000;
        total = total.saturating_add(crit_dmg.saturating_mul(stacks));
    }
    total
}

/// All alive entity uids on the side opposite `owner_uid`. Side is
/// determined by uid sign (positive = attacker, negative = defender);
/// this matches the existing convention used by
/// `extend_with_magic_circle_enemy_skills`.
fn iter_hostile_alive_uids(fight: &sonettobuf::Fight, owner_uid: i64) -> Vec<i64> {
    let owner_sign = owner_uid.signum();
    let mut out = Vec::new();
    if let Some(side) = &fight.attacker {
        for e in side.entitys.iter().chain(side.sub_entitys.iter()) {
            if let Some(uid) = e.uid
                && uid.signum() != owner_sign
                && e.current_hp.unwrap_or(0) > 0
            {
                out.push(uid);
            }
        }
    }
    if let Some(side) = &fight.defender {
        for e in side.entitys.iter().chain(side.sub_entitys.iter()) {
            if let Some(uid) = e.uid
                && uid.signum() != owner_sign
                && e.current_hp.unwrap_or(0) > 0
            {
                out.push(uid);
            }
        }
    }
    out
}

fn entity_will_die_from(ctx: &FightContext<'_>, carrier_uid: i64, damage: i32) -> bool {
    let Some(victim) = get_entity(ctx.fight, carrier_uid) else {
        return false;
    };
    let hp = victim.current_hp.unwrap_or(0);
    let shield = victim.shield_value.unwrap_or(0);
    let after_shield = damage.saturating_sub(shield);
    after_shield > 0 && hp - after_shield <= 0
}
