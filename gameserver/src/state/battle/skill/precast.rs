use sonettobuf::Fight;

use crate::state::battle::{
    buff_actions::add_passive_skills::for_each_add_passive_skill_id_for_entity,
    manager::fight_data_mgr::Managers,
    skill::{
        cache::SKILL_CACHE, euphoria::resolve_skill_effect_id_for_entity, source_kind,
        targets::get_entity,
    },
    types::{behavior::BehaviorType, condition::ConditionType},
};

fn targeted_psychube_entry_equip_id(skill_id: i32) -> Option<i32> {
    match source_kind::classify(skill_id) {
        source_kind::SkillSource::PsychubeSkill { equip_id }
        | source_kind::SkillSource::PortraySkill { equip_id, .. }
            if matches!(equip_id, 1528 | 1538) =>
        {
            Some(equip_id)
        }
        _ => None,
    }
}

fn is_self_buff_prep_skill(skill_id: i32) -> bool {
    let cfg = config::configs::get();
    cfg.skill_effect
        .iter()
        .find(|s| s.id == skill_id)
        .map(|row| {
            let expect_behavior = format!("1#{}", skill_id);
            let is_standard_prep =
                row.condition1.starts_with("660008#1") && row.behavior1 == expect_behavior;
            let is_targeted_psychube_entry = targeted_psychube_entry_equip_id(skill_id).is_some()
                && row.behavior1 == expect_behavior
                && (row.condition1.starts_with("660008#1") || row.condition1.starts_with("1104#"));
            is_standard_prep || is_targeted_psychube_entry
        })
        .unwrap_or(false)
}

fn find_self_buff_prep_skills(passive_skills: &[i32]) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    for &sid in passive_skills {
        if sid <= 0 {
            continue;
        }

        // Keep psychube prep skills on the exact passive-surface variant that
        // the fight snapshot already carries. Promoting `433811 -> 433815`
        // over-fires later portray branches that are out of scope here.
        let can_promote_variant = targeted_psychube_entry_equip_id(sid).is_none();

        if is_self_buff_prep_skill(sid) && !out.contains(&sid) {
            out.push(sid);
        }

        if !can_promote_variant {
            continue;
        }

        let mut best_for_sid: Option<i32> = None;
        for delta in 1..=20 {
            let candidate = sid + delta;
            if is_self_buff_prep_skill(candidate) {
                best_for_sid = Some(best_for_sid.map_or(candidate, |cur| cur.max(candidate)));
            }
        }
        if let Some(candidate) = best_for_sid
            && !out.contains(&candidate)
        {
            out.push(candidate);
        }
    }
    out
}

pub(crate) fn collect_precast_skills_for_caster(
    fight: &Fight,
    managers: &Managers,
    caster_uid: i64,
) -> Vec<i32> {
    let mut passive_candidates: Vec<i32> = get_entity(fight, caster_uid)
        .map(|e| e.passive_skill.clone())
        .unwrap_or_default();

    // Buff feature 865(AddPassiveSkills) contributes virtual passive skills while buff is active.
    for instance in managers.buff_mgr.get(caster_uid) {
        for_each_add_passive_skill_id_for_entity(fight, caster_uid, instance.buff_id, |skill_id| {
            if !passive_candidates.contains(&skill_id) {
                passive_candidates.push(skill_id);
            }
        });
    }

    find_self_buff_prep_skills(&passive_candidates)
}

pub(crate) fn infer_precast_per_decr_seed_cap(
    fight: &Fight,
    managers: &Managers,
    caster_uid: i64,
    prep_skill_ids: &[i32],
) -> Option<i32> {
    let active = managers.buff_mgr.get(caster_uid);
    let mut best: Option<i32> = None;

    for &skill_id in prep_skill_ids {
        let effect_id = resolve_skill_effect_id_for_entity(fight, caster_uid, skill_id);
        let Some(rows) = SKILL_CACHE.get(&effect_id) else {
            continue;
        };
        for row in rows {
            let is_per_decr = matches!(row.condition, ConditionType::PerDecrExPoint { .. });
            if !is_per_decr {
                continue;
            }
            let BehaviorType::AddBuff { buff_id, .. } = row.behavior else {
                continue;
            };
            let cap = active
                .iter()
                .find(|b| b.buff_id == buff_id)
                .map(|b| b.layer.max(b.stacks))
                .unwrap_or(0);
            if cap <= 0 {
                // Some prep skills are injected by AddPassiveSkills features.
                // When no direct target buff exists yet, seed from the source
                // passive buff's current stack/layer value.
                let mut source_cap = 0;
                for source in active {
                    let mut matched = false;
                    for_each_add_passive_skill_id_for_entity(
                        fight,
                        caster_uid,
                        source.buff_id,
                        |sid| {
                            if !matched && sid == skill_id {
                                matched = true;
                            }
                        },
                    );
                    if matched {
                        source_cap = source.layer.max(source.stacks).max(0);
                        if source_cap > 0 {
                            break;
                        }
                    }
                }
                if source_cap <= 0 {
                    continue;
                }
                best = Some(best.map_or(source_cap, |v| v.min(source_cap)));
                continue;
            }
            best = Some(best.map_or(cap, |v| v.min(cap)));
        }
    }

    best
}
