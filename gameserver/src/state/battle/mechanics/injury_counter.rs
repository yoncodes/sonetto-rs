use crate::state::battle::{
    event_queue::{BattleEvent, HostEventAccumulator},
    fight_step::{ActEffectBuilder, effect_container_step, wrap_step},
    skill::{cache::resolve_skill_effect_id, get_entity},
    types::effects::EffectType,
};
use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, Fight, FightStep, fight_step};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

static ROUND_INJURY_COUNT: Lazy<Mutex<HashMap<(i32, i32), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static ROUND_INJURY_INDEX: Lazy<Mutex<HashMap<(i32, i32), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
const CARD_HOST_MARKER_SKILL_IDS: [i32; 3] = [308801821, 308802011, 308801611];

pub(crate) fn increment_round_injury_count(battle_id: i32, team_type: i32) {
    if battle_id == 0 || team_type <= 0 {
        return;
    }
    let mut tracker = ROUND_INJURY_COUNT.lock().unwrap();
    let entry = tracker.entry((battle_id, team_type)).or_insert(0);
    *entry = entry.saturating_add(1);
}

pub(crate) fn get_round_injury_count(battle_id: i32, team_type: i32) -> i32 {
    ROUND_INJURY_COUNT
        .lock()
        .unwrap()
        .get(&(battle_id, team_type))
        .copied()
        .unwrap_or(0)
}

pub(crate) fn sync_round_injury_index(battle_id: i32, team_type: i32, round_index: i32) {
    if battle_id == 0 || team_type <= 0 {
        return;
    }
    let key = (battle_id, team_type);
    let mut index_map = ROUND_INJURY_INDEX.lock().unwrap();
    let last_round = index_map.get(&key).copied();
    if last_round != Some(round_index) {
        ROUND_INJURY_COUNT.lock().unwrap().insert(key, 0);
        index_map.insert(key, round_index);
    }
}

pub(crate) fn track_team_injury_count(fight: &Fight, step: &FightStep) {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 {
        return;
    }
    let mut stack = vec![step];
    while let Some(cur) = stack.pop() {
        let mut damage_targets = HashSet::new();
        for effect in &cur.act_effect {
            if effect.fight_step.is_some() {
                continue;
            }
            let effect_type = effect.effect_type.unwrap_or(0);
            if is_damage_effect_type(effect_type)
                && let Some(target_uid) = effect.target_id
            {
                damage_targets.insert(target_uid);
            }
        }
        for effect in &cur.act_effect {
            if let Some(nested) = effect.fight_step.as_ref() {
                stack.push(nested);
                continue;
            }
            let Some(target_uid) = effect.target_id else {
                continue;
            };
            let Some(target) = crate::state::battle::skill::get_entity(fight, target_uid) else {
                continue;
            };
            let Some(team_type) = target.team_type else {
                continue;
            };
            let effect_type = effect.effect_type.unwrap_or(0);
            let is_damage = is_damage_effect_type(effect_type);
            let is_current_hp_change = effect_type == EffectType::CurrentHpChange as i32;
            let hp_loss = is_damage
                || (!damage_targets.contains(&target_uid) && is_current_hp_change)
                || effect_type == EffectType::AverageLife as i32;
            if hp_loss {
                increment_round_injury_count(battle_id, team_type);
            }
        }
    }
}

pub(crate) fn find_round_injury_skill_rate_params(
    fight: &Fight,
    holder_uid: i64,
    output_skill_id: i32,
) -> Option<(i32, i32)> {
    let cfg = config::configs::get();
    let holder = crate::state::battle::skill::get_entity(fight, holder_uid)?;
    for passive_skill_id in &holder.passive_skill {
        if *passive_skill_id <= 0 {
            continue;
        }
        let effect_id = resolve_skill_effect_id(*passive_skill_id);
        let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
            continue;
        };
        let conditions = [
            skill.condition1.as_str(),
            skill.condition2.as_str(),
            skill.condition3.as_str(),
            skill.condition4.as_str(),
            skill.condition5.as_str(),
            skill.condition6.as_str(),
            skill.condition7.as_str(),
            skill.condition8.as_str(),
            skill.condition9.as_str(),
            skill.condition10.as_str(),
        ];
        let behaviors = [
            skill.behavior1.as_str(),
            skill.behavior2.as_str(),
            skill.behavior3.as_str(),
            skill.behavior4.as_str(),
            skill.behavior5.as_str(),
            skill.behavior6.as_str(),
            skill.behavior7.as_str(),
            skill.behavior8.as_str(),
            skill.behavior9.as_str(),
            skill.behavior10.as_str(),
        ];

        for (raw_condition, raw_behavior) in conditions.into_iter().zip(behaviors) {
            let rate = raw_behavior
                .trim()
                .strip_prefix("10001#")
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if rate <= 0 || raw_condition.trim().is_empty() {
                continue;
            }

            let mut matches_skill = false;
            let mut cap = None;
            for segment in raw_condition.split('&') {
                let parts: Vec<&str> = segment.split('#').collect();
                let Some(cond_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
                    continue;
                };
                let cond_type = cfg
                    .skill_behavior_condition
                    .iter()
                    .find(|c| c.id == cond_id)
                    .map(|c| c.r#type.as_str())
                    .unwrap_or("");
                match cond_type {
                    "UseSkillId" | "CanUseSkill" => {
                        if parts
                            .iter()
                            .skip(1)
                            .filter_map(|v| v.trim().parse::<i32>().ok())
                            .any(|sid| sid == output_skill_id)
                        {
                            matches_skill = true;
                        }
                    }
                    "TeamInjuryCountRound" => {
                        cap = parts.get(1).and_then(|v| v.trim().parse::<i32>().ok());
                    }
                    _ => {}
                }
            }
            if matches_skill {
                return Some((cap.unwrap_or(20), rate));
            }
        }
    }
    None
}

pub(crate) fn apply_round_injury_skill_bonus(
    fight: &Fight,
    skill_effects: &mut [ActEffect],
    holder_uid: i64,
    output_skill_id: i32,
    primary_target_uid: i64,
    stacks: i32,
    rate_per_stack: i32,
) {
    let Some(skill_step) = find_nested_skill_step_mut(skill_effects, output_skill_id) else {
        return;
    };
    let total_bonus_rate = stacks.saturating_mul(rate_per_stack).max(0);
    if total_bonus_rate <= 0 {
        return;
    }

    let counter_insert_at = skill_step
        .act_effect
        .iter()
        .position(|e| is_damage_effect_type(e.effect_type.unwrap_or(0)))
        .unwrap_or(skill_step.act_effect.len());
    skill_step.act_effect.insert(
        counter_insert_at,
        ActEffectBuilder::fight_counter(holder_uid, stacks),
    );

    let mut first_bonus_amount = 0;
    let mut first_damage_target = 0;
    for effect in &mut skill_step.act_effect {
        if !is_damage_effect_type(effect.effect_type.unwrap_or(0)) {
            continue;
        }
        let base = effect.effect_num.unwrap_or(0).max(0);
        if base <= 0 {
            continue;
        }
        let bonus = base.saturating_mul(total_bonus_rate) / 800;
        let boosted = base.saturating_add(bonus).max(1);
        effect.effect_num = Some(boosted);
        if let Some(hurt) = effect.hurt_info.as_mut() {
            hurt.damage = Some(boosted);
            if hurt.reduce_hp.unwrap_or(0) > 0 {
                hurt.reduce_hp = Some(boosted);
            }
        }
        if first_damage_target == 0 {
            first_damage_target = effect.target_id.unwrap_or(0);
            first_bonus_amount = bonus;
        }
    }

    let dead_effects = collect_dead_effects_after_damage(fight, &skill_step.act_effect);
    if !dead_effects.is_empty() {
        skill_step.act_effect.extend(dead_effects);
    }

    let additional_target = if primary_target_uid != 0 {
        primary_target_uid
    } else {
        first_damage_target
    };
    if additional_target != 0 && first_bonus_amount > 0 {
        skill_step
            .act_effect
            .push(ActEffectBuilder::additional_damage_crit(
                additional_target,
                first_bonus_amount,
            ));
    }
}

pub(crate) fn count_team_injury_effects_in_effects(
    fight: &Fight,
    effects: &[ActEffect],
    team_type: i32,
) -> i32 {
    count_team_injury_effects_in_effects_with_parent(fight, effects, team_type, None)
}

pub(crate) fn count_team_injury_effects_in_effects_with_parent(
    fight: &Fight,
    effects: &[ActEffect],
    team_type: i32,
    _parent_act_id: Option<i32>,
) -> i32 {
    let mut count = 0;
    let mut damage_targets = HashSet::new();
    for effect in effects {
        if effect.fight_step.is_some() {
            continue;
        }
        let effect_type = effect.effect_type.unwrap_or(0);
        if is_damage_effect_type(effect_type)
            && let Some(target_uid) = effect.target_id
        {
            damage_targets.insert(target_uid);
        }
    }
    for effect in effects {
        if let Some(step) = effect.fight_step.as_ref() {
            count += count_team_injury_effects_in_effects_with_parent(
                fight,
                &step.act_effect,
                team_type,
                step.act_id,
            );
            continue;
        }
        let Some(target_uid) = effect.target_id else {
            continue;
        };
        let Some(target) = crate::state::battle::skill::get_entity(fight, target_uid) else {
            continue;
        };
        if target.team_type != Some(team_type) {
            continue;
        }
        let effect_type = effect.effect_type.unwrap_or(0);
        let is_damage = is_damage_effect_type(effect_type);
        let is_current_hp_change = effect_type == EffectType::CurrentHpChange as i32;
        let hp_loss = is_damage
            || (!damage_targets.contains(&target_uid) && is_current_hp_change)
            || effect_type == EffectType::AverageLife as i32;
        if hp_loss {
            count += 1;
        }
    }
    count
}

pub(crate) fn find_nested_skill_step_mut(
    effects: &mut [ActEffect],
    act_id: i32,
) -> Option<&mut FightStep> {
    for effect in effects {
        if let Some(step) = effect.fight_step.as_mut() {
            if step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.act_id == Some(act_id)
            {
                return Some(step);
            }
            for nested in &mut step.act_effect {
                if let Some(inner) = nested.fight_step.as_mut()
                    && inner.act_type == Some(fight_step::ActType::Skill as i32)
                    && inner.act_id == Some(act_id)
                {
                    return Some(inner);
                }
            }
        }
    }
    None
}

pub(crate) fn collect_dead_effects_after_damage(
    fight: &Fight,
    effects: &[ActEffect],
) -> Vec<ActEffect> {
    let mut states: HashMap<i64, (i32, i32)> = HashMap::new();
    let mut dead_targets: HashSet<i64> = effects
        .iter()
        .filter(|effect| effect.effect_type == Some(EffectType::Dead as i32))
        .filter_map(|effect| effect.target_id)
        .collect();
    let mut killed_in_order = Vec::new();

    for effect in effects {
        if !is_damage_effect_type(effect.effect_type.unwrap_or(0)) {
            continue;
        }
        let Some(target_id) = effect.target_id else {
            continue;
        };
        if target_id == 0 || dead_targets.contains(&target_id) {
            continue;
        }
        let Some(entity) = crate::state::battle::skill::get_entity(fight, target_id) else {
            continue;
        };
        let current_hp = entity.current_hp.unwrap_or(0);
        if current_hp <= 0 {
            dead_targets.insert(target_id);
            continue;
        }

        let (hp, shield) = states
            .entry(target_id)
            .or_insert((current_hp, entity.shield_value.unwrap_or(0)));
        let damage = effect.effect_num.unwrap_or(0).max(0);
        let shield_absorbed = damage.min(*shield);
        let hp_damage = damage.saturating_sub(shield_absorbed);
        *shield = shield.saturating_sub(shield_absorbed);
        *hp = hp.saturating_sub(hp_damage);

        if *hp <= 0 {
            dead_targets.insert(target_id);
            killed_in_order.push(target_id);
        }
    }

    killed_in_order
        .into_iter()
        .flat_map(|target_id| {
            [
                ActEffectBuilder::dead(target_id),
                ActEffectBuilder::remove_entity_cards(target_id, Some(1)),
            ]
        })
        .collect()
}

pub(crate) fn find_card_host_injury_marker_params(
    fight: &Fight,
    host_from_uid: i64,
) -> Option<(i64, i32)> {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 || host_from_uid == 0 {
        return None;
    }
    let team_type = get_entity(fight, host_from_uid)?.team_type?;
    let injury_count = get_round_injury_count(battle_id, team_type).max(0);
    if injury_count <= 0 {
        return None;
    }
    Some((
        find_round_injury_holder_uid(fight, team_type)?,
        injury_count,
    ))
}

pub(crate) fn inject_card_host_injury_markers(
    host_step: &FightStep,
    fight: &Fight,
    holder_uid: i64,
    injury_count: i32,
    accumulator: &mut HostEventAccumulator,
) -> Vec<(usize, ActEffect)> {
    if injury_count <= 0 || holder_uid == 0 {
        return Vec::new();
    }
    let Some(caster_team_type) = host_step
        .from_id
        .and_then(|uid| get_entity(fight, uid))
        .and_then(|entity| entity.team_type)
    else {
        return Vec::new();
    };

    let mut markers = Vec::with_capacity(4);
    for (idx, effect) in host_step.act_effect.iter().enumerate() {
        if let Some(injured_uid) = flat_host_injury_marker_uid(fight, effect, caster_team_type) {
            let marker = build_card_host_injury_marker(injured_uid, holder_uid, injury_count);
            accumulator.push_injury(BattleEvent::SerializedActEffect {
                effect: marker.clone(),
            });
            markers.push((idx, marker));
        }

        let nested_marker_uid = effect
            .fight_step
            .as_ref()
            .and_then(|step| nested_host_injury_marker_uid(fight, step, caster_team_type));
        if let Some(injured_uid) = nested_marker_uid {
            let marker = build_card_host_injury_marker(injured_uid, holder_uid, injury_count);
            accumulator.push_injury(BattleEvent::SerializedActEffect {
                effect: marker.clone(),
            });
            markers.push((idx + 1, marker));
        }
    }
    markers
}

pub(crate) fn is_damage_effect_type(effect_type: i32) -> bool {
    matches!(
        EffectType::from(effect_type),
        EffectType::Damage
            | EffectType::Crit
            | EffectType::DamageExtra
            | EffectType::OriginDamage
            | EffectType::OriginCrit
            | EffectType::DamageFromAbsorb
            | EffectType::DamageFromLostHp
            | EffectType::EnchantBurnDamage
            | EffectType::DamageShareHp
            | EffectType::DeadlyPoisonOriginDamage
            | EffectType::DeadlyPoisonOriginCrit
            | EffectType::AdditionalDamage
            | EffectType::AdditionalDamageCrit
            | EffectType::ShareHurt
            | EffectType::EnchantDepresseDamage
    )
}

fn find_round_injury_holder_uid(fight: &Fight, team_type: i32) -> Option<i64> {
    fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
        .filter(|entity| entity.team_type == Some(team_type))
        .find_map(|entity| {
            let uid = entity.uid?;
            has_round_injury_counter_passive(fight, uid).then_some(uid)
        })
}

fn has_round_injury_counter_passive(fight: &Fight, holder_uid: i64) -> bool {
    let cfg = config::configs::get();
    let Some(holder) = get_entity(fight, holder_uid) else {
        return false;
    };
    for passive_skill_id in &holder.passive_skill {
        if *passive_skill_id <= 0 {
            continue;
        }
        let effect_id = resolve_skill_effect_id(*passive_skill_id);
        let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
            continue;
        };
        let conditions = [
            skill.condition1.as_str(),
            skill.condition2.as_str(),
            skill.condition3.as_str(),
            skill.condition4.as_str(),
            skill.condition5.as_str(),
            skill.condition6.as_str(),
            skill.condition7.as_str(),
            skill.condition8.as_str(),
            skill.condition9.as_str(),
            skill.condition10.as_str(),
        ];
        let behaviors = [
            skill.behavior1.as_str(),
            skill.behavior2.as_str(),
            skill.behavior3.as_str(),
            skill.behavior4.as_str(),
            skill.behavior5.as_str(),
            skill.behavior6.as_str(),
            skill.behavior7.as_str(),
            skill.behavior8.as_str(),
            skill.behavior9.as_str(),
            skill.behavior10.as_str(),
        ];
        for (raw_condition, raw_behavior) in conditions.into_iter().zip(behaviors) {
            let rate = raw_behavior
                .trim()
                .strip_prefix("10001#")
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            if rate <= 0 || raw_condition.trim().is_empty() {
                continue;
            }
            for segment in raw_condition.split('&') {
                let parts: Vec<&str> = segment.split('#').collect();
                let Some(cond_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
                    continue;
                };
                let cond_type = cfg
                    .skill_behavior_condition
                    .iter()
                    .find(|c| c.id == cond_id)
                    .map(|c| c.r#type.as_str())
                    .unwrap_or("");
                if cond_type == "TeamInjuryCountRound" {
                    return true;
                }
            }
        }
    }
    false
}

fn build_card_host_injury_marker(
    injured_uid: i64,
    holder_uid: i64,
    injury_count: i32,
) -> ActEffect {
    wrap_step(effect_container_step(
        injured_uid,
        injured_uid,
        0,
        vec![ActEffectBuilder::fight_counter(holder_uid, injury_count)],
    ))
}

fn flat_host_injury_marker_uid(
    fight: &Fight,
    effect: &ActEffect,
    caster_team_type: i32,
) -> Option<i64> {
    if effect.fight_step.is_some() || !is_damage_effect_type(effect.effect_type.unwrap_or(0)) {
        return None;
    }
    let target_uid = effect.target_id?;
    if get_entity(fight, target_uid).and_then(|entity| entity.team_type) != Some(caster_team_type) {
        return None;
    }
    Some(target_uid)
}

fn nested_host_injury_marker_uid(
    fight: &Fight,
    step: &FightStep,
    caster_team_type: i32,
) -> Option<i64> {
    if step.act_type != Some(fight_step::ActType::Skill as i32) {
        return None;
    }
    let act_id = step.act_id.unwrap_or(0);
    if !CARD_HOST_MARKER_SKILL_IDS.contains(&act_id) {
        return None;
    }
    let injured_uid = step.from_id?;
    if get_entity(fight, injured_uid).and_then(|entity| entity.team_type) != Some(caster_team_type)
    {
        return None;
    }
    step_contains_team_damage(fight, &step.act_effect, caster_team_type).then_some(injured_uid)
}

fn step_contains_team_damage(fight: &Fight, effects: &[ActEffect], team_type: i32) -> bool {
    for effect in effects {
        if let Some(step) = effect.fight_step.as_ref() {
            if step_contains_team_damage(fight, &step.act_effect, team_type) {
                return true;
            }
            continue;
        }
        let Some(target_uid) = effect.target_id else {
            continue;
        };
        if !is_damage_effect_type(effect.effect_type.unwrap_or(0)) {
            continue;
        }
        if get_entity(fight, target_uid).and_then(|entity| entity.team_type) == Some(team_type) {
            return true;
        }
    }
    false
}
