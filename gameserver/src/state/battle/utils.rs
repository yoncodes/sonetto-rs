use super::{manager::buff_mgr::BuffMgr, skill::get_entity, types::career::CareerType};

use sonettobuf::{ActEffect, Fight, FightEntityInfo};

//skill_behaviour table
pub enum VfxConfig {
    Heal = 20001,
    Damage = 30006,
    Moxie = 20002,
}

#[allow(dead_code)]
pub enum EffectTag {
    None = 0,
    Skill = 1,
    SkillEffect = 2,
    Buff = 3,
    Additional = 4,
    AbsorbHurt = 5,
    ShareHurt = 6,
}

#[allow(dead_code)]
pub enum DamageType {
    Reality = 1,
    Mental = 2,
}

pub fn buff_has_bloodpool(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    if buff.features.is_empty() {
        return false;
    }
    buff.features.split('|').any(|entry| {
        let act_id: i32 = entry
            .split('#')
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        cfg.buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "BloodPoolTag")
            .unwrap_or(false)
    })
}

/// Sum the target's active `RealHurtFix(519)` feature permille.
/// Tuesday's "Genesis DMG Taken +X%" debuff encodes here as `519#150`,
/// `519#250`, `519#350`, etc.
pub fn target_real_hurt_fix_permille(buff_mgr: &BuffMgr, target_uid: i64) -> i32 {
    let cfg = config::configs::get();
    let mut total = 0;
    for instance in buff_mgr.get(target_uid) {
        let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id) else {
            continue;
        };
        for entry in buff.features.split('|') {
            let mut parts = entry.split('#');
            let act_id = parts
                .next()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if act_id != 519 {
                continue;
            }
            let permille = parts
                .next()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            total += permille;
        }
    }
    total
}

pub fn apply_real_hurt_fix(buff_mgr: &BuffMgr, target_uid: i64, base_damage: i32) -> i32 {
    let multiplier_permille = 1000 + target_real_hurt_fix_permille(buff_mgr, target_uid);
    base_damage
        .max(0)
        .saturating_mul(multiplier_permille.max(0))
        / 1000
}

pub fn for_each_buff_feature_chain(buff_id: i32, mut f: impl FnMut(&str, &[&str])) {
    let cfg = config::configs::get();
    let mut stack = vec![buff_id];
    let mut seen = std::collections::HashSet::new();

    while let Some(current_buff_id) = stack.pop() {
        if current_buff_id <= 0 || !seen.insert(current_buff_id) {
            continue;
        }
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == current_buff_id) else {
            continue;
        };
        if buff_cfg.features.is_empty() {
            continue;
        }

        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id = parts
                .first()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");

            if act_type == "SubBuff" {
                for raw in parts.iter().skip(1) {
                    for piece in raw.split(',') {
                        let Ok(child_buff_id) = piece.trim().parse::<i32>() else {
                            continue;
                        };
                        if child_buff_id > 0 {
                            stack.push(child_buff_id);
                        }
                    }
                }
            }

            f(act_type, &parts);
        }
    }
}

pub fn buff_get_act_common_params(buff_id: i32) -> String {
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return String::new();
    };
    for entry in buff.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts
            .first()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let is_overflow = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "ExPointOverflowBank")
            .unwrap_or(false);
        if is_overflow {
            return format!("{}#0", act_id);
        }
    }
    String::new()
}

/// Check if any active buff on the caster has AttrOnlyCalDamageReplaceAttr feature.
/// Returns (source_attr, replace_attr, permille) if found.
/// Feature format: buff_act_id#source_attr#replace_attr#permille
/// Sum flat Attr bonus for a given attr_id from all active buffs on entity.
/// Handles buff_act type "Attr": feature format "100#attr_id#amount"
/// Also handles "AttrByLostHp" (853): scales bonus by lost HP ratio.
pub fn get_attr_bonus(
    buff_mgr: &BuffMgr,
    fight: &sonettobuf::Fight,
    entity_uid: i64,
    attr_id: i32,
) -> i32 {
    let cfg = config::configs::get();
    let mut total = 0i32;
    let mut stacked_key_total: std::collections::HashMap<String, i32> =
        std::collections::HashMap::new();
    let mut stacked_key_layer_bonus: std::collections::HashMap<String, i32> =
        std::collections::HashMap::new();
    let mut handled_stacked_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for buff in buff_mgr.get(entity_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        let Some(buff_type_cfg) = cfg.skill_bufftype.iter().find(|t| t.id == buff_cfg.type_id)
        else {
            continue;
        };
        let include_type = buff_type_cfg
            .include_types
            .split('#')
            .next()
            .unwrap_or_default()
            .trim();
        if matches!(include_type, "10" | "12" | "14" | "15") {
            let sign_key = format!("{}__{}", buff_cfg.features, buff_cfg.type_id);
            *stacked_key_total.entry(sign_key.clone()).or_insert(0) += 1;
            if buff.layer > 0 {
                *stacked_key_layer_bonus.entry(sign_key).or_insert(0) += buff.layer - 1;
            }
        }
    }

    for buff in buff_mgr.get(entity_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        let buff_type_cfg = cfg.skill_bufftype.iter().find(|t| t.id == buff_cfg.type_id);
        let include_type = buff_type_cfg
            .and_then(|t| t.include_types.split('#').next())
            .unwrap_or_default()
            .trim();
        let is_stacked = matches!(include_type, "10" | "12" | "14" | "15");
        let sign_key = format!("{}__{}", buff_cfg.features, buff_cfg.type_id);
        let stack_multiplier = if is_stacked {
            if !handled_stacked_keys.insert(sign_key.clone()) {
                continue;
            }
            let base = stacked_key_total.get(&sign_key).copied().unwrap_or(1);
            let layer_bonus = stacked_key_layer_bonus.get(&sign_key).copied().unwrap_or(0);
            (base + layer_bonus).max(1)
        } else {
            (if buff.stacks > 0 { buff.stacks } else { 1 }).max(1)
        };

        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id: i32 = match parts.first().and_then(|v| v.trim().parse().ok()) {
                Some(id) => id,
                None => continue,
            };
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");

            match act_type {
                "Attr" => {
                    // format: act_id#attr_id#amount
                    let feat_attr: i32 = match parts.get(1).and_then(|v| v.trim().parse().ok()) {
                        Some(v) => v,
                        None => continue,
                    };
                    if feat_attr != attr_id {
                        continue;
                    }
                    let amount: i32 = parts
                        .get(2)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if is_stacked && attr_id == 206 {
                        // Live-like behavior for stacked DropDmg-style shields:
                        // apply one instance per hit, not full stack multiplication.
                        total += amount;
                    } else {
                        total += amount * stack_multiplier;
                    }
                }
                "AttrOnlyCalDamageAttack"
                | "AttrOnlyCalDamageBeAttacked"
                | "AttrOnlyCalDamageAttackType"
                | "AttrOnlyCalDamageBeAttackedType" => {
                    // format typically: act_id#attr_id#amount#[optional_type]
                    let feat_attr: i32 = match parts.get(1).and_then(|v| v.trim().parse().ok()) {
                        Some(v) => v,
                        None => continue,
                    };
                    if feat_attr != attr_id {
                        continue;
                    }
                    let amount: i32 = parts
                        .get(2)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if is_stacked && attr_id == 206 {
                        total += amount;
                    } else {
                        total += amount * stack_multiplier;
                    }
                }
                "AttrByLostHp" => {
                    // format: act_id#source_attr#attr_ids(comma)#amounts(comma)#max_stacks
                    // attrs and amounts are comma-separated pairs
                    let attr_ids_str = match parts.get(2) {
                        Some(v) => *v,
                        None => continue,
                    };
                    let amounts_str = match parts.get(3) {
                        Some(v) => *v,
                        None => continue,
                    };
                    let max_stacks: i32 = parts
                        .get(4)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1);

                    let attr_ids: Vec<i32> = attr_ids_str
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();
                    let amounts: Vec<i32> = amounts_str
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();

                    let pos = match attr_ids.iter().position(|&a| a == attr_id) {
                        Some(p) => p,
                        None => continue,
                    };
                    let base_amount = match amounts.get(pos) {
                        Some(&a) => a,
                        None => continue,
                    };

                    // Scale by lost HP ratio * max_stacks
                    if let Some(entity) = get_entity(fight, entity_uid) {
                        let max_hp = entity.attr.as_ref().and_then(|a| a.hp).unwrap_or(1).max(1);
                        let cur_hp = entity.current_hp.unwrap_or(max_hp);
                        let lost_permille = ((max_hp - cur_hp).max(0) * 1000) / max_hp;
                        let stacks = (lost_permille * max_stacks / 1000).min(max_stacks);
                        total += base_amount * stacks * stack_multiplier;
                    }
                }
                _ => {}
            }
        }
    }
    total
}

pub fn get_attr_replace_damage(buff_mgr: &BuffMgr, caster_uid: i64) -> Option<(i32, i32, i32)> {
    let cfg = config::configs::get();
    for buff in buff_mgr.get(caster_uid) {
        let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff.buff_id) else {
            continue;
        };
        for entry in buff_cfg.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id: i32 = match parts.first().and_then(|v| v.trim().parse().ok()) {
                Some(id) => id,
                None => continue,
            };
            let is_replace = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| {
                    a.r#type == "AttrOnlyCalDamageReplaceAttr"
                        || a.r#type == "AttrOnlyCalDamageReplaceAttrADCreator"
                })
                .unwrap_or(false);
            if is_replace {
                let source_attr = match parts.get(1).and_then(|v| v.trim().parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                let replace_attr = match parts.get(2).and_then(|v| v.trim().parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                let permille = match parts.get(3).and_then(|v| v.trim().parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                return Some((source_attr, replace_attr, permille));
            }
        }
    }
    None
}

pub fn get_exclude_buff_effects(buff_mgr: &BuffMgr, target: i64, buff_id: i32) -> Vec<ActEffect> {
    let mut effects = Vec::new();
    let cfg = config::configs::get();

    tracing::warn!(
        "get_exclude_buff_effects buff_id={} target={} existing_buffs={:?}",
        buff_id,
        target,
        buff_mgr
            .get(target)
            .iter()
            .map(|b| b.buff_id)
            .collect::<Vec<_>>()
    );

    // look up buff_type by the buff's typeId
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id);
    let type_id = buff.map(|b| b.type_id).unwrap_or(buff_id);

    let Some(buff_type) = cfg.skill_bufftype.iter().find(|t| t.id == type_id) else {
        return effects;
    };

    if buff_type.exclude_types.is_empty() {
        return effects;
    }

    let exclude_str = buff_type.exclude_types.trim_start_matches("2#");
    let excluded_type_ids: Vec<i32> = exclude_str
        .split(['，', ','])
        .filter_map(|v| v.trim().parse().ok())
        .collect();

    for instance in buff_mgr.get(target) {
        if excluded_type_ids.contains(&instance.type_id) {
            effects.push(
                crate::state::battle::fight_step::ActEffectBuilder::buff_del(
                    target,
                    instance.uid,
                    instance.buff_id,
                    instance.from_uid,
                ),
            );
        }
    }

    effects
}

pub fn find_entity(fight: &Fight, uid: i64) -> Option<&FightEntityInfo> {
    if let Some(a) = &fight.attacker {
        for e in a.entitys.iter().chain(a.sub_entitys.iter()) {
            if e.uid == Some(uid) {
                return Some(e);
            }
        }
    }
    if let Some(d) = &fight.defender {
        for e in d.entitys.iter().chain(d.sub_entitys.iter()) {
            if e.uid == Some(uid) {
                return Some(e);
            }
        }
    }
    None
}

/// Resolve the runtime UID of an attacker-side entity by its stable hero_id.
/// Returns None when no entity matches the current battle composition.
pub fn find_uid_by_hero_id(fight: &Fight, hero_id: i32) -> Option<i64> {
    fight
        .attacker
        .as_ref()?
        .entitys
        .iter()
        .chain(fight.attacker.as_ref()?.sub_entitys.iter())
        .find(|entity| entity.model_id == Some(hero_id))
        .and_then(|entity| entity.uid)
}

/// Resolve the runtime UID of a defender-side entity by its stable hero_id.
/// Returns None when no entity matches the current battle composition.
#[allow(dead_code)]
pub fn find_defender_uid_by_hero_id(fight: &Fight, hero_id: i32) -> Option<i64> {
    fight
        .defender
        .as_ref()?
        .entitys
        .iter()
        .chain(fight.defender.as_ref()?.sub_entitys.iter())
        .find(|entity| entity.model_id == Some(hero_id))
        .and_then(|entity| entity.uid)
}

pub fn check_career_restraint(attacker_career: i32, defender_career: i32) -> bool {
    let attacker = CareerType::from(attacker_career);
    let defender = CareerType::from(defender_career);

    matches!(
        (attacker, defender),
        (CareerType::Mineral, CareerType::Beast) | // Mineral beats Beast
        (CareerType::Star, CareerType::Mineral) |  // Star beats Mineral
        (CareerType::Plant, CareerType::Star) |    // Plant beats Star
        (CareerType::Beast, CareerType::Plant) |   // Beast beats Plant
        (CareerType::Spirit, CareerType::Intellect) | // Spirit beats Intellect
        (CareerType::Intellect, CareerType::Spirit) // Intellect beats Spirit
    )
}

pub fn career_damage_multiplier(attacker_career: i32, defender_career: i32) -> i32 {
    let attacker = CareerType::from(attacker_career);
    let defender = CareerType::from(defender_career);

    if check_career_restraint(attacker as i32, defender as i32) {
        1300 // 130% = +30% bonus
    } else if check_career_restraint(defender as i32, attacker as i32) {
        700 // 70% = -30% penalty
    } else {
        1000 // 100% = no modifier
    }
}

#[allow(dead_code)]
pub fn modify_hero_attr(entity: &mut FightEntityInfo, attr_id: i32, amount_permille: i32) {
    use super::types::attr::AttrId;
    let attr = entity.attr.get_or_insert_with(Default::default);
    match AttrId::from(attr_id) {
        Some(AttrId::Hp) => attr.hp = Some(attr.hp.unwrap_or(0) + amount_permille),
        Some(AttrId::Attack) => {
            attr.attack = Some(attr.attack.unwrap_or(0) * (1000 + amount_permille) / 1000)
        }
        Some(AttrId::Cri) => {}    // stat only, no direct attr field
        Some(AttrId::CriDmg) => {} // stat only
        Some(AttrId::AddDmg) => {} // stat only
        _ => tracing::warn!("modify_hero_attr: unhandled attr_id={}", attr_id),
    }
}

pub fn find_entity_mut(fight: &mut Fight, uid: i64) -> Option<&mut FightEntityInfo> {
    for team in fight.attacker.iter_mut().chain(fight.defender.iter_mut()) {
        if let Some(entity) = team
            .entitys
            .iter_mut()
            .find(|entity| entity.uid == Some(uid))
        {
            return Some(entity);
        }
        if let Some(entity) = team
            .sub_entitys
            .iter_mut()
            .find(|entity| entity.uid == Some(uid))
        {
            return Some(entity);
        }
    }
    None
}

pub fn gains_standard_action_ex(fight: &Fight, uid: i64) -> bool {
    get_entity(fight, uid)
        .and_then(|entity| entity.ex_point_type)
        .and_then(super::types::ex_point::ExPointType::from_i32)
        .map(|ex_type| ex_type.gains_from_standard_actions())
        .unwrap_or(false)
}
