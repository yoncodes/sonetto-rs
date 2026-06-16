use super::RefreshPolicy;
use crate::state::battle::types::buff::IncludeType;

pub fn derive_refresh_policy(bt: Option<&config::skill_bufftype::SkillBufftype>) -> RefreshPolicy {
    let Some(bt) = bt else {
        return RefreshPolicy::UpdateInPlace;
    };
    let include_base: i32 = bt
        .include_types
        .split('#')
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    let has_exclude_types = !bt.exclude_types.is_empty();
    match IncludeType::from(include_base) {
        Some(IncludeType::Unique) => RefreshPolicy::ReplaceOnSelfRefresh,
        Some(IncludeType::Stacked) if has_exclude_types => RefreshPolicy::ReplaceOnExcludedOverlap,
        _ => RefreshPolicy::UpdateInPlace,
    }
}

pub fn is_stacked_include_type(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    let Some(buff_type_cfg) = cfg.skill_bufftype.iter().find(|t| t.id == buff_cfg.type_id) else {
        return false;
    };
    let include_type = buff_type_cfg
        .include_types
        .split('#')
        .next()
        .unwrap_or_default()
        .trim();
    matches!(include_type, "10" | "12" | "14" | "15")
}

pub fn is_poison_family(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    buff_cfg.features.split('|').any(|entry| {
        entry
            .split('#')
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
            .is_some_and(|act_id| matches!(act_id, 803 | 844))
    })
}

#[allow(dead_code)]
pub fn is_drop_dmg_attr_buff(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
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
            .unwrap_or_default();
        let is_attr_like = matches!(
            act_type,
            "Attr"
                | "AttrOnlyCalDamageAttack"
                | "AttrOnlyCalDamageBeAttacked"
                | "AttrOnlyCalDamageAttackType"
                | "AttrOnlyCalDamageBeAttackedType"
        );
        if !is_attr_like {
            continue;
        }
        let Some(attr_id) = parts.get(1).and_then(|v| v.trim().parse::<i32>().ok()) else {
            continue;
        };
        if attr_id == 206 {
            return true;
        }
    }
    false
}

pub fn uses_single_uid_layer_refresh(buff_id: i32) -> bool {
    buff_id == 30091120
}

pub fn uses_distinct_dot_carrier_instances(buff_id: i32) -> bool {
    matches!(buff_id, 30980111 | 30980132)
}
