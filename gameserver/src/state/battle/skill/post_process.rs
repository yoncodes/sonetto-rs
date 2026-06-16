use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use super::super::{
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    types::effects::EffectType,
};
use super::super::effect::behavior::damage::{
    CURE_TYPE_ID, DUALITY_POTION_BUFF_ID, build_sotheby_holder_consume_steps,
};

pub(super) fn is_damage_effect_type(effect_type: i32) -> bool {
    effect_type == EffectType::Damage as i32
        || effect_type == EffectType::Crit as i32
        || effect_type == EffectType::DamageExtra as i32
        || effect_type == EffectType::OriginDamage as i32
        || effect_type == EffectType::OriginCrit as i32
        || effect_type == EffectType::AdditionalDamage as i32
        || effect_type == EffectType::AdditionalDamageCrit as i32
        || effect_type == EffectType::FixedDamage as i32
        || effect_type == EffectType::DamageFromAbsorb as i32
        || effect_type == EffectType::DamageFromLostHp as i32
        || effect_type == EffectType::EnchantBurnDamage as i32
        || effect_type == EffectType::EnchantDepresseDamage as i32
        || effect_type == EffectType::DeadlyPoisonOriginDamage as i32
        || effect_type == EffectType::DeadlyPoisonOriginCrit as i32
}

pub(super) fn is_bonus_damage_config_effect(config_effect: i32) -> bool {
    matches!(config_effect, 60038 | 60039 | 60040)
}

/// Consume `AttrOnlyCalDamageAttack` buffs on `entity_uid` and return inline 162 steps.
pub(super) fn consume_attr_only_damage_buffs(
    managers: &mut Managers,
    entity_uid: i64,
) -> Vec<ActEffect> {
    let cfg = config::configs::get();
    let mut out = Vec::new();

    let to_consume: Vec<(i64, i32, i64)> = managers
        .buff_mgr
        .get(entity_uid)
        .iter()
        .filter_map(|b| {
            let buff_cfg = cfg.skill_buff.get(b.buff_id)?;
            let has_attr_only = buff_cfg.features.split('|').any(|entry| {
                let act_id = entry
                    .split('#')
                    .next()
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .unwrap_or(0);
                cfg.buff_act
                    .get(act_id)
                    .map(|a| a.r#type == "AttrOnlyCalDamageAttack")
                    .unwrap_or(false)
            });
            if has_attr_only { Some((b.uid, b.buff_id, b.from_uid)) } else { None }
        })
        .collect();

    for (buff_uid, buff_id, from_uid) in to_consume {
        managers.buff_mgr.remove_by_uid(entity_uid, buff_uid);
        let inner = FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            from_id: Some(from_uid),
            to_id: Some(entity_uid),
            act_id: Some(buff_id),
            act_effect: vec![ActEffectBuilder::buff_del(entity_uid, buff_uid, buff_id, from_uid)],
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        };
        out.push(ActEffectBuilder::skill_wrapper(inner));
    }

    out
}

/// Inject Sotheby Duality Potion consume steps for basic/upgraded-basic skill lanes.
/// Only fires for skill_id 30090111 or 30090112.
pub(super) fn inject_sotheby_consume(
    effects: &mut Vec<ActEffect>,
    fight: &Fight,
    managers: &mut Managers,
    caster_uid: i64,
    skill_id: i32,
) {
    if !matches!(skill_id, 30090111 | 30090112) {
        return;
    }
    if !effects.iter().any(|e| e.effect_type.map(is_damage_effect_type).unwrap_or(false)) {
        return;
    }
    let Some(holder) = managers
        .buff_mgr
        .find_instance_by_buff_id(caster_uid, DUALITY_POTION_BUFF_ID)
        .cloned()
    else {
        return;
    };

    let mut hostile_targets: Vec<i64> = Vec::new();
    for effect in effects.iter() {
        if !is_damage_effect_type(effect.effect_type.unwrap_or(0)) {
            continue;
        }
        if let Some(ti) = effect.target_id
            && ti.signum() != caster_uid.signum()
            && !hostile_targets.contains(&ti)
        {
            hostile_targets.push(ti);
        }
    }
    if hostile_targets.is_empty() {
        return;
    }

    let stack_count = holder.layer.max(1);
    let consume_steps = build_sotheby_holder_consume_steps(
        fight,
        caster_uid,
        &hostile_targets,
        CURE_TYPE_ID,
        &holder,
        stack_count,
        false,
    );
    effects.extend(consume_steps);
    managers.buff_mgr.remove_by_uid(caster_uid, holder.uid);
}

/// Lift damage payloads out of self-nested same-act_id FightStep wrappers.
pub(super) fn normalize_nested_steps(effects: Vec<ActEffect>, skill_id: i32) -> Vec<ActEffect> {
    let mut out = Vec::with_capacity(effects.len());
    for mut effect in effects {
        if effect.effect_type == Some(EffectType::FightStep as i32)
            && let Some(step) = effect.fight_step.take()
        {
            if step.act_id == Some(skill_id)
                && step.act_effect.iter().any(|e| e.effect_type.is_some_and(is_damage_effect_type))
            {
                out.extend(step.act_effect);
                continue;
            }
            effect.fight_step = Some(step);
        }
        out.push(effect);
    }
    out
}
