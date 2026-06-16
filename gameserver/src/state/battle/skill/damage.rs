use super::super::{
    manager::buff_mgr::BuffMgr,
    types::attr::AttrId,
    utils::{
        VfxConfig, career_damage_multiplier, check_career_restraint, get_attr_bonus,
        get_attr_replace_damage,
    },
};
use super::targets::get_entity;
use crate::state::battle::event_queue::{BattleEvent, serialize_leaf_event};
use sonettobuf::{
    fight_hurt_info::DamageFromType,
    {ActEffect, Fight, effect_type_enum::EffectType},
};

fn fight_const_i32(id: i32, default: i32) -> i32 {
    config::configs::get()
        .fight_const
        .get(id)
        .and_then(|r| r.value.parse::<i32>().ok())
        .unwrap_or(default)
}

fn deterministic_roll_permille(seed: i64, caster_uid: i64, target_uid: i64, skill_id: i32) -> i32 {
    // Stable but simple combat roll that better tracks live-like "mostly non-crit,
    // occasional crit" distribution for identical replay inputs.
    let x =
        seed as i128 + caster_uid as i128 * 31 + target_uid as i128 * 17 + skill_id as i128 * 13;
    let m = x.rem_euclid(1000);
    m as i32
}

fn has_behavioral_crit_override(buff_mgr: &BuffMgr, entity_uid: i64, want: &str) -> bool {
    let cfg = config::configs::get();
    for buff in buff_mgr.get(entity_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let Some(act_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
                continue;
            };
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");
            if act_type == want {
                return true;
            }
        }
    }
    false
}

/// Per-hero base `exAttr.(dropDmg, addDmg)` keyed by hero_id (= entity
/// `model_id`). Mirrors the `heroMO.exAttr` payload the LIVE server
/// pushes to the client at login (sourced from
/// `assets/static/heros/hero_list.json::exAttr`); the fight setup does
/// NOT serialize these into `FightEntityInfo.attr`, so the values are
/// hardcoded here as a stopgap.
///
/// TODO: replace with a proper login-time loader once the server-side
/// account-state pipeline lands. Until then, extend the table with
/// additional heroes as new fixtures land. Returns `(0, 0)` for unknown
/// hero_ids so buff/pending lanes remain authoritative.
fn static_ex_attr_for_hero(hero_id: i32) -> (i32, i32) {
    match hero_id {
        // (drop_dmg, add_dmg)
        3009 => (175, 185), // Sotheby
        3063 => (155, 165), // Pickles
        3080 => (185, 60),  // Kakania
        3088 => (155, 165), // Semmelweis
        3098 => (25, 70),   // Tuesday
        3104 => (155, 165), // Willow
        3114 => (50, 80),   // Recoleta
        3120 => (45, 85),   // Nautika
        3125 => (155, 165), // Rubuska
        3126 => (155, 165), // Sentinel
        _ => (0, 0),
    }
}

fn static_ex_attr_drop_dmg(fight: &Fight, uid: i64) -> i32 {
    let Some(entity) = get_entity(fight, uid) else {
        return 0;
    };
    let Some(model_id) = entity.model_id else {
        return 0;
    };
    static_ex_attr_for_hero(model_id).0
}

fn static_ex_attr_add_dmg(fight: &Fight, uid: i64) -> i32 {
    let Some(entity) = get_entity(fight, uid) else {
        return 0;
    };
    let Some(model_id) = entity.model_id else {
        return 0;
    };
    static_ex_attr_for_hero(model_id).1
}

pub fn should_crit_hit(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    entity_mgr: &crate::state::battle::manager::entity_mgr::EntityMgr,
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
) -> bool {
    // if has_behavioral_crit_override(buff_mgr, caster_uid, "MustCrit")
    //     || has_behavioral_crit_override(buff_mgr, caster_uid, "MustCritBuff")
    // {
    //     return true;
    // }
    // if has_behavioral_crit_override(buff_mgr, caster_uid, "CantCrit")
    //     || has_behavioral_crit_override(buff_mgr, target_uid, "CantCrit")
    // {
    //     return false;
    // }

    let Some(caster) = get_entity(fight, caster_uid) else {
        return false;
    };
    let Some(target) = get_entity(fight, target_uid) else {
        return false;
    };

    // Crit/Recrit are permille-style attrs from buff features (Attr ids 201/202).
    let mut crit_permille = entity_mgr.sum_attr_bonus(caster_uid, 201);
    let recrit_permille = entity_mgr.sum_attr_bonus(target_uid, 202);

    // CharacterModel.lua:
    // add_cri = floor(technic * const11 / (const13 + target_level*const14))
    let technic = caster
        .attr
        .as_ref()
        .and_then(|a| a.technic)
        .unwrap_or(0)
        .max(0);
    let target_level = target.level.unwrap_or(1).max(1);
    let technic_critical_ratio = fight_const_i32(11, 100);
    let _technic_critical_damage_ratio = fight_const_i32(12, 150);
    let technic_correct_const = fight_const_i32(13, 300);
    let technic_target_level_ratio = fight_const_i32(14, 0);
    let denom = (technic_correct_const + target_level * technic_target_level_ratio).max(1);
    let technic_add = (technic * technic_critical_ratio) / denom;
    crit_permille += technic_add;

    let effective = (crit_permille - recrit_permille).clamp(0, 1000);
    if effective <= 0 {
        return false;
    }

    let seed = fight.cur_round.unwrap_or(1) as i64
        + fight.version.unwrap_or(0) as i64
        + fight.battle_id.unwrap_or(0) as i64;
    deterministic_roll_permille(seed, caster_uid, target_uid, skill_id) < effective
}

#[allow(clippy::too_many_arguments)]
pub fn calculate_damage(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    entity_mgr: &crate::state::battle::manager::entity_mgr::EntityMgr,
    caster_uid: i64,
    target_uid: i64,
    base_param: i32,
    _skill_id: i32,
    is_crit: bool,
) -> Vec<ActEffect> {
    let Some(caster) = get_entity(fight, caster_uid) else {
        return vec![];
    };
    let Some(target) = get_entity(fight, target_uid) else {
        return vec![];
    };
    let Some(caster_attr) = caster.attr.as_ref() else {
        return vec![];
    };
    let target_defense = target
        .attr
        .as_ref()
        .and_then(|a| a.defense)
        .unwrap_or(50)
        .saturating_add(entity_mgr.sum_attr_bonus(target_uid, 103));
    // let caster_attack = if let Some((source_attr, replace_attr, permille)) =
    //     get_attr_replace_damage(buff_mgr, caster_uid)
    // {
    //     if AttrId::from(source_attr) == Some(AttrId::Attack) {
    //         let replace_val = match AttrId::from(replace_attr) {
    //             Some(AttrId::Hp) => caster_attr.hp.unwrap_or(0),
    //             Some(AttrId::CurrentHp) => caster.current_hp.unwrap_or(0),
    //             Some(AttrId::Attack) => caster_attr.attack.unwrap_or(0),
    //             _ => caster_attr.attack.unwrap_or(0),
    //         };
    //         replace_val.saturating_mul(permille) / 1000
    //             + pending_attr_bonus(pending_attr, caster_uid, 102)
    //     } else {
    //         caster_attr
    //             .attack
    //             .unwrap_or(100)
    //             .saturating_add(pending_attr_bonus(pending_attr, caster_uid, 102))
    //     }
    // } else {
    //     caster_attr
    //         .attack
    //         .unwrap_or(100)
    //         .saturating_add(pending_attr_bonus(pending_attr, caster_uid, 102))
    // };
    let caster_attack = caster_attr
        .attack
        .unwrap_or(100)
        .saturating_add(entity_mgr.sum_attr_bonus(caster_uid, 102));
    // Live-like mitigation is ratio-based instead of flat subtraction.
    let attack_contribution = if caster_attack <= 0 {
        0
    } else {
        ((caster_attack as i64 * 1000) / (1000 + target_defense.max(0) as i64)) as i32
    };
    let skill_multiplier = base_param as f32 / 1000.0;
    let crit_dmg = entity_mgr.sum_attr_bonus(caster_uid, 203);
    let crit_def = entity_mgr.sum_attr_bonus(target_uid, 204);
    let crit_multiplier = if is_crit {
        // fight_const id 12 is stored as percent (e.g. 150), convert to permille lane (1500)
        // to stay compatible with the existing damage multiplier pipeline.
        let base_crit_permille = fight_const_i32(12, 150).saturating_mul(10);
        (base_crit_permille + crit_dmg - crit_def).max(0) as f32 / 1000.0
    } else {
        1.0
    };

    // career restraint
    let caster_career = caster.career.unwrap_or(0);
    let target_career = target.career.unwrap_or(0);
    let restraint_mult = career_damage_multiplier(caster_career, target_career) as f32 / 1000.0;

    let dmg =
        ((attack_contribution as f32) * skill_multiplier * crit_multiplier * restraint_mult) as i32;
    let dmg = dmg.max(1);

    // AddDmg (205) / DropDmg (206) combine buff-granted Attr modifiers
    // with the hero's base exAttr.addDmg/dropDmg (the login-time
    // hero_list.json payload). The fight setup does not serialize the
    // base exAttr into `FightEntityInfo.attr`, so `static_ex_attr_*`
    // hardcodes them per-hero as a stopgap until the login-time loader
    // is wired in.
    let add_dmg = entity_mgr.sum_attr_bonus(caster_uid, 205);
    let drop_dmg = entity_mgr.sum_attr_bonus(target_uid, 206)
        + static_ex_attr_drop_dmg(fight, target_uid);
    let add_dmg = add_dmg + static_ex_attr_add_dmg(fight, caster_uid);
    let dmg_mult = (1000 + add_dmg - drop_dmg).max(0) as f32 / 1000.0;
    let dmg = ((dmg as f32) * dmg_mult) as i32;
    let dmg = dmg.max(1);

    let restraint = check_career_restraint(caster_career, target_career);
    let primary_effect = if is_crit || restraint {
        EffectType::Crit as i32
    } else {
        EffectType::Damage as i32
    };

    vec![serialize_leaf_event(BattleEvent::Damage {
        target: target_uid,
        amount: dmg,
        is_crit: primary_effect == EffectType::Crit as i32,
        hurt_info: sonettobuf::FightHurtInfo {
            damage: Some(dmg),
            reduce_hp: Some(0),
            reduce_shield: Some(0),
            career_restraint: Some(restraint),
            critical: Some(is_crit),
            assassinate: Some(false),
            hurt_effect: Some(primary_effect),
            damage_from_type: Some(DamageFromType::Skill as i32),
            config_effect: Some(-1),
            buff_act_id: Some(0),
            buff_uid: Some(0),
            effect_id: Some(0),
            skill_id: Some(0),
            from_uid: Some(caster_uid),
        },
        from: caster_uid,
        skill_id: None,
    })]
}

pub fn calculate_heal(
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    base_param: i32,
    is_crit: bool,
) -> Option<ActEffect> {
    let caster = get_entity(fight, caster_uid)?;
    let caster_attack = caster.attr.as_ref()?.attack.unwrap_or(100);
    let heal_contribution = (caster_attack as f32 * (base_param as f32 / 100.0)) as i32;
    let final_heal = (base_param + heal_contribution).max(1);
    Some(heal_effect(target_uid, final_heal, is_crit))
}

pub fn calculate_heal_by_two_attr(
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    missing_percent: i32,
    caster_hp_percent: i32,
) -> Vec<ActEffect> {
    let Some(caster) = get_entity(fight, caster_uid) else {
        return vec![];
    };
    let Some(target) = get_entity(fight, target_uid) else {
        return vec![];
    };

    let target_current = target.current_hp.unwrap_or(0);
    let target_max = target
        .attr
        .as_ref()
        .and_then(|a| a.hp)
        .unwrap_or(target_current);
    let missing_hp = (target_max - target_current).max(0);
    let heal_from_missing = (missing_hp as f32 * (missing_percent as f32 / 1000.0)) as i32;
    let caster_max = caster.attr.as_ref().and_then(|a| a.hp).unwrap_or(1000);
    let heal_from_caster = (caster_max as f32 * (caster_hp_percent as f32 / 1000.0)) as i32;
    let total = (heal_from_missing + heal_from_caster).max(1);

    vec![heal_effect(target_uid, total, false)]
}

pub fn heal_effect(target_id: i64, heal: i32, is_crit: bool) -> ActEffect {
    let mut effect = if is_crit {
        serialize_leaf_event(BattleEvent::HealCrit {
            target: target_id,
            amount: heal,
            from: 0,
        })
    } else {
        serialize_leaf_event(BattleEvent::Heal {
            target: target_id,
            amount: heal,
            from: 0,
        })
    };
    effect.config_effect = Some(VfxConfig::Heal as i32);
    effect
}
