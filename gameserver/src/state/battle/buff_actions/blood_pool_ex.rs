//! Handler for buff_act 1021 BloodPoolCountAddExPoint — fires EX-point gain when bloodtithe value crosses configured thresholds.

use once_cell::sync::Lazy;
use sonettobuf::{Fight, FightStep, fight_step};
use std::{collections::HashMap, sync::Mutex};

use crate::state::battle::{
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::ActEffectBuilder,
    heroes::rubuska,
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr},
    mechanics::bloodtithe::BloodtitheState,
    utils::find_entity,
};

pub const BUFF_ACT_ID: i32 = 1021;

static BLOOD_POOL_EX_TRACKER: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static BLOOD_POOL_EX_ACCUM_TRACKER: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn buff_get_blood_pool_ex_point_params(buff_id: i32) -> Option<(i32, i32)> {
    super::first_feature_match(buff_id, |act_type, parts| match act_type {
        "BloodPoolCountAddExPoint" => {
            let threshold = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            let amount = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            (threshold > 0 && amount > 0).then_some((threshold, amount))
        }
        "ExPointOverflowBank" => {
            let threshold = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            (threshold > 0).then_some((threshold, 1))
        }
        _ => None,
    })
}

fn buff_uses_blood_pool_gain_accum(buff_id: i32) -> bool {
    super::find_feature_parts(buff_id, "BloodPoolCountAddExPoint").is_some()
}

pub fn build_blood_pool_ex_point_step(
    bloodtithe: &mut BloodtitheState,
    fight: &Fight,
    buff_mgr: &BuffMgr,
    entity_mgr: &mut EntityMgr,
) -> Option<FightStep> {
    if !bloodtithe.initialized {
        return None;
    }
    if bloodtithe.get_value(1) == 0 {
        return None;
    }

    let uids: Vec<i64> = fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
        .unwrap_or_default();

    let mut outer_effects = Vec::new();
    let battle_key = fight.battle_id.unwrap_or(0);
    let mut tracker = BLOOD_POOL_EX_TRACKER.lock().unwrap();

    for uid in uids {
        let Some(team_type) = find_entity(fight, uid).and_then(|e| e.team_type) else {
            continue;
        };
        let total = bloodtithe.get_value(team_type);
        let model_id = find_entity(fight, uid).and_then(|e| e.model_id);
        for instance in buff_mgr.get(uid) {
            if buff_uses_blood_pool_gain_accum(instance.buff_id) {
                continue;
            }
            if rubuska::uses_shadow_cloak_overflow_tracker(model_id, instance.buff_id) {
                // Rubuska's 806 tracker is Shadow Cloak-driven, not bloodpool-driven.
                continue;
            }
            let Some((threshold, amount)) = buff_get_blood_pool_ex_point_params(instance.buff_id)
            else {
                continue;
            };
            let tracker_key = (battle_key, instance.uid);
            let Some(previous_total) = tracker.get(&tracker_key).copied() else {
                tracker.insert(tracker_key, total);
                continue;
            };
            let gain = ((total / threshold) - (previous_total / threshold)).max(0) * amount;
            tracker.insert(tracker_key, total);
            if gain <= 0 {
                continue;
            }

            let mut act_effect = Vec::new();
            for _ in 0..gain {
                act_effect.push(ActEffectBuilder::effect_none_with_buff_act(
                    uid,
                    instance.buff_id,
                    BUFF_ACT_ID,
                ));
                let mut queue = EventQueue::new();
                queue.push(BattleEvent::ExPointChange {
                    target: uid,
                    delta: 1,
                });
                let mut synthetic_fight = Fight::default();
                let mut synthetic_buff_mgr = BuffMgr::new();
                let mut event_ctx = EventContext {
                    fight: &mut synthetic_fight,
                    buff_mgr: &mut synthetic_buff_mgr,
                    entity_mgr,
                    bloodtithe,
                };
                act_effect.extend(drain_to_fight_steps(queue.drain(), &mut event_ctx));
            }

            outer_effects.push(ActEffectBuilder::skill_wrapper_without_num(FightStep {
                act_type: Some(fight_step::ActType::Effect.into()),
                from_id: Some(uid),
                to_id: Some(uid),
                act_id: Some(instance.buff_id),
                act_effect,
                card_index: Some(0),
                support_hero_id: Some(0),
                fake_timeline: Some(false),
                real_skill_type: Some(0),
                real_skin_id: Some(0),
            }));
        }
    }

    if outer_effects.is_empty() {
        return None;
    }

    Some(FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        act_effect: outer_effects,
        ..Default::default()
    })
}

pub fn build_blood_pool_gain_ex_point_step(
    bloodtithe: &BloodtitheState,
    fight: &Fight,
    buff_mgr: &BuffMgr,
    entity_mgr: &mut EntityMgr,
    gains_by_team: &[(i32, i32)],
    _gains_by_skill_team: &[(i32, i32, i32)],
) -> Option<FightStep> {
    let battle_key = fight.battle_id.unwrap_or(0);
    if battle_key == 0 {
        return None;
    }

    let gain_lookup: HashMap<i32, i32> = gains_by_team
        .iter()
        .copied()
        .filter(|(_, gain)| *gain > 0)
        .collect();
    if gain_lookup.is_empty() {
        return None;
    }

    let uids: Vec<i64> = fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
        .unwrap_or_default();

    let mut outer_effects = Vec::new();
    let mut accum_tracker = BLOOD_POOL_EX_ACCUM_TRACKER.lock().unwrap();
    for uid in uids {
        let Some(team_type) = find_entity(fight, uid).and_then(|e| e.team_type) else {
            continue;
        };
        let Some(delta) = gain_lookup.get(&team_type).copied() else {
            continue;
        };
        let current_total = bloodtithe.get_value(team_type).max(0);
        for instance in buff_mgr.get(uid) {
            if !buff_uses_blood_pool_gain_accum(instance.buff_id) {
                continue;
            }
            let Some((threshold, amount)) = buff_get_blood_pool_ex_point_params(instance.buff_id)
            else {
                continue;
            };
            let tracker_key = (battle_key, instance.uid);
            let entry = accum_tracker
                .entry(tracker_key)
                .or_insert_with(|| current_total.rem_euclid(threshold));
            *entry += delta;
            let gain = (*entry / threshold).max(0) * amount;
            if gain <= 0 {
                continue;
            }
            *entry %= threshold;

            let mut act_effect = Vec::new();
            for _ in 0..gain {
                act_effect.push(ActEffectBuilder::effect_none_with_buff_act(
                    uid,
                    instance.buff_id,
                    BUFF_ACT_ID,
                ));
                let mut queue = EventQueue::new();
                queue.push(BattleEvent::ExPointChange {
                    target: uid,
                    delta: 1,
                });
                let mut synthetic_fight = Fight::default();
                let mut synthetic_buff_mgr = BuffMgr::new();
                let mut synthetic_bloodtithe = BloodtitheState::new();
                let mut event_ctx = EventContext {
                    fight: &mut synthetic_fight,
                    buff_mgr: &mut synthetic_buff_mgr,
                    entity_mgr,
                    bloodtithe: &mut synthetic_bloodtithe,
                };
                act_effect.extend(drain_to_fight_steps(queue.drain(), &mut event_ctx));
            }

            outer_effects.push(ActEffectBuilder::skill_wrapper_without_num(FightStep {
                act_type: Some(fight_step::ActType::Effect.into()),
                from_id: Some(uid),
                to_id: Some(uid),
                act_id: Some(instance.buff_id),
                act_effect,
                card_index: Some(0),
                support_hero_id: Some(0),
                fake_timeline: Some(false),
                real_skill_type: Some(0),
                real_skin_id: Some(0),
            }));
        }
    }

    if outer_effects.is_empty() {
        return None;
    }

    Some(FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        act_effect: outer_effects,
        ..Default::default()
    })
}

// Consumed by battle_gen's replay bootstrap; clippy can't see cross-crate callers.
#[allow(dead_code)]
pub fn seed_blood_pool_ex_tracker(bloodtithe: &BloodtitheState, fight: &Fight, buff_mgr: &BuffMgr) {
    if !bloodtithe.initialized {
        return;
    }

    let battle_key = fight.battle_id.unwrap_or(0);
    if battle_key == 0 {
        return;
    }

    let mut tracker = BLOOD_POOL_EX_TRACKER.lock().unwrap();
    let mut accum_tracker = BLOOD_POOL_EX_ACCUM_TRACKER.lock().unwrap();
    for uid in fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().filter_map(|e| e.uid))
    {
        let Some(team_type) = find_entity(fight, uid).and_then(|e| e.team_type) else {
            continue;
        };
        let total = bloodtithe.get_value(team_type);
        let model_id = find_entity(fight, uid).and_then(|e| e.model_id);
        for instance in buff_mgr.get(uid) {
            if buff_uses_blood_pool_gain_accum(instance.buff_id) {
                if let Some((threshold, _)) = buff_get_blood_pool_ex_point_params(instance.buff_id)
                {
                    accum_tracker.insert((battle_key, instance.uid), total.rem_euclid(threshold));
                }
                continue;
            }
            if rubuska::uses_shadow_cloak_overflow_tracker(model_id, instance.buff_id) {
                continue;
            }
            if buff_get_blood_pool_ex_point_params(instance.buff_id).is_none() {
                continue;
            }
            tracker.insert((battle_key, instance.uid), total);
        }
    }
}
