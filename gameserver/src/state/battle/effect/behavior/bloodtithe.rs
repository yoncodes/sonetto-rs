use rand::rngs::StdRng;
use sonettobuf::{ActEffect, Fight, FightHurtInfo, fight_hurt_info::DamageFromType};

use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::manager::{
    buff_mgr::BuffMgr as EventBuffMgr, entity_mgr::EntityMgr as EventExPointMgr,
};
use crate::state::battle::{
    event::Event,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::ActEffectBuilder,
    heroes::{nautika, rubuska, semmelweis},
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr, fight_data_mgr::Managers},
    mechanics::{
        Mechanics,
        bloodtithe::{BloodtitheState, bloodtithe_add_to_pool, bloodtithe_value_change},
    },
    skill::damage::calculate_damage,
    skill::{SkillExecutor, targets::get_entity},
    types::effects::EffectType,
    utils::apply_real_hurt_fix,
};

fn attr_value(entity: &sonettobuf::FightEntityInfo, attr_id: i32) -> i32 {
    let attr = entity.attr.as_ref();
    match attr_id {
        100 => entity.current_hp.unwrap_or(0),
        101 => attr.and_then(|a| a.hp).unwrap_or(0),
        102 => attr.and_then(|a| a.attack).unwrap_or(0),
        103 => attr.and_then(|a| a.defense).unwrap_or(0),
        _ => attr.and_then(|a| a.attack).unwrap_or(0),
    }
}

fn burn_params(buff_id: i32) -> Option<(i32, i32, i32)> {
    if buff_id <= 0 {
        return None;
    }
    let cfg = config::configs::get();
    let buff_cfg = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    for entry in buff_cfg.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let Some(act_id) = parts.first().and_then(|v| v.trim().parse::<i32>().ok()) else {
            continue;
        };
        if cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "Burn")
            .unwrap_or(false)
        {
            let rate = parts
                .get(1)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let attr_id = parts
                .get(2)
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            return Some((act_id, rate, attr_id));
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn lost_life(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    bloodtithe: &mut BloodtitheState,
    caster_uid: i64,
    target: i64,
    mode: i32,
    _attr_id: i32,
    permille: i32,
    behavior_id: i32,
    skill_id: i32,
    floor_permille: i32,
) -> Vec<ActEffect> {
    let mut effects = Vec::new();

    let entity = get_entity(fight, target);
    let max_hp = entity
        .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
        .unwrap_or(0);
    let current_hp = entity.and_then(|e| e.current_hp).unwrap_or(0);
    let burn = (mode == 1).then(|| burn_params(skill_id)).flatten();
    let loss = if let Some((_, burn_rate, burn_attr_id)) = burn {
        let burn_rate = if burn_rate > 0 { burn_rate } else { permille };

        let from_damage = calculate_damage(
            fight,
            buff_mgr,
            &EntityMgr::default(),
            caster_uid,
            target,
            burn_rate,
            skill_id,
            false,
        )
        .into_iter()
        .find(|e| {
            matches!(
                e.effect_type,
                Some(t)
                    if t == EffectType::Damage as i32
                        || t == EffectType::Crit as i32
                        || t == EffectType::OriginDamage as i32
                        || t == EffectType::OriginCrit as i32
            )
        })
        .and_then(|e| e.effect_num)
        .unwrap_or(0);

        if from_damage > 0 {
            from_damage
        } else {
            let source = get_entity(fight, caster_uid)
                .map(|e| attr_value(e, burn_attr_id))
                .unwrap_or(0);
            source * permille / 1000
        }
    } else if rubuska::is_basic_self_loss(skill_id) && target == caster_uid && _attr_id == 100 {
        rubuska::basic_self_loss_amount(fight, target, permille)
    } else if mode == 1 {
        current_hp * permille / 1000
    } else {
        max_hp * permille / 10000
    };

    let min_hp = if floor_permille > 0 {
        (max_hp * floor_permille / 1000).max(1)
    } else {
        0
    };
    let actual_loss = loss.min((current_hp - min_hp).max(0));

    if actual_loss == 0 {
        return effects;
    }

    if let Some((act_id, _, _)) = burn {
        let buff_uid = buff_mgr
            .get(target)
            .iter()
            .find(|b| b.buff_id == skill_id || b.type_id == skill_id)
            .map(|b| b.uid)
            .unwrap_or(0);

        effects.push(ActEffectBuilder::burn(
            target,
            skill_id,
            (act_id > 0).then_some(act_id),
        ));

        let damage = apply_real_hurt_fix(buff_mgr, target, actual_loss);
        effects.push(ActEffectBuilder::origin_damage_with_hurt(
            target,
            damage,
            (act_id > 0).then_some(act_id),
            FightHurtInfo {
                damage: Some(damage),
                reduce_hp: Some(0),
                reduce_shield: Some(0),
                career_restraint: Some(false),
                critical: Some(false),
                assassinate: Some(false),
                hurt_effect: Some(EffectType::OriginDamage as i32),
                damage_from_type: Some(DamageFromType::Buff as i32),
                config_effect: Some(0),
                buff_act_id: (act_id > 0).then_some(act_id),
                buff_uid: (buff_uid > 0).then_some(buff_uid as i32),
                effect_id: Some(0),
                skill_id: Some(0),
                from_uid: Some(caster_uid),
            },
        ));
    } else {
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::Damage {
            target,
            amount: actual_loss,
            is_crit: false,
            hurt_info: FightHurtInfo {
                damage: Some(actual_loss),
                reduce_hp: Some(0),
                hurt_effect: Some(EffectType::Damage as i32),
                damage_from_type: Some(DamageFromType::SkillEffect as i32),
                config_effect: Some(behavior_id),
                effect_id: Some(skill_id),
                skill_id: Some(skill_id),
                from_uid: Some(caster_uid),
                ..Default::default()
            },
            from: caster_uid,
            skill_id: Some(skill_id),
        });

        let mut synthetic_fight = Fight::default();
        let mut synthetic_buff_mgr = EventBuffMgr::new();
        let mut synthetic_entity_mgr = EventExPointMgr::default();
        let drained = {
            let mut event_ctx = EventContext {
                fight: &mut synthetic_fight,
                buff_mgr: &mut synthetic_buff_mgr,
                entity_mgr: &mut synthetic_entity_mgr,
                bloodtithe,
            };
            drain_to_fight_steps(queue.drain(), &mut event_ctx)
        };
        effects.extend(drained);
    }

    let team_type = get_entity(fight, target).and_then(|e| e.team_type);
    let model_id = get_entity(fight, target).and_then(|e| e.model_id);

    // Battle2 parity: Semmelweis Ultimate body emits four visible 335 packets even when
    // our replay-seeded bloodpool cap is already saturated. Mirror the live packet lane
    // here and let replay-time 335 application advance the authoritative pool value.
    if skill_id == *semmelweis::TIER_IV_ULT_SKILL_ID {
        let manual_gain = semmelweis::ult_manual_gain(model_id);
        if manual_gain > 0 {
            effects.push(bloodtithe_add_to_pool(target, manual_gain));
            return effects;
        }
    }
    let preview_gain = team_type.and_then(|team_type| {
        let mut preview = bloodtithe.clone();
        preview.on_hp_lost(target, team_type, actual_loss)
    });
    if let Some(new_value) = preview_gain {
        if nautika::is_nautika(model_id) {
            effects.push(nautika::faith_gain_one(target));
        }
        effects.push(bloodtithe_add_to_pool(target, new_value));
    }

    effects
}

pub fn pool_max_change(
    _fight: &Fight,
    bloodtithe: &mut BloodtitheState,
    _target: i64,
    amount: i32,
) -> Vec<ActEffect> {
    let mut queue = EventQueue::new();
    queue.push(BattleEvent::BloodpoolMaxChange {
        team_type: 1,
        max: amount,
    });
    let mut synthetic_fight = Fight::default();
    let mut synthetic_buff_mgr = EventBuffMgr::new();
    let mut synthetic_entity_mgr = EventExPointMgr::default();
    let drained = {
        let mut event_ctx = EventContext {
            fight: &mut synthetic_fight,
            buff_mgr: &mut synthetic_buff_mgr,
            entity_mgr: &mut synthetic_entity_mgr,
            bloodtithe,
        };
        drain_to_fight_steps(queue.drain(), &mut event_ctx)
    };
    bloodtithe.pending_effects.extend(drained);
    vec![]
}

pub fn pool_value_change(
    fight: &Fight,
    bloodtithe: &mut BloodtitheState,
    target: i64,
    amount: i32,
) -> Vec<ActEffect> {
    bloodtithe.add_initial_gain(1, amount);

    let model_id = get_entity(fight, target).and_then(|e| e.model_id);
    if nautika::is_nautika(model_id) {
        bloodtithe
            .pending_effects
            .push(nautika::faith_gain_one(target));
    }
    bloodtithe
        .pending_effects
        .push(bloodtithe_value_change(target, amount, 1));
    vec![]
}

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    targets: Vec<i64>,
    entity_uid: i64,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
) -> Vec<Event> {
    let target = targets.into_iter().next().unwrap_or(0);
    let amount: i32 = raw
        .split('#')
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let effects = match beh_type {
        BehaviourType::_60190BloodPoolMaxChange => {
            pool_max_change(fight, &mut mechanics.bloodtithe, target, amount)
        }
        BehaviourType::_60191BloodPoolValueChange => {
            pool_value_change(fight, &mut mechanics.bloodtithe, target, amount)
        }
        BehaviourType::_60199ConsumeBloodPoolHeal => {
            tracing::warn!("unimplemented bloodtithe behaviour: {:?}", beh_type);
            vec![]
        }
        _ => vec![],
    };
    effects
        .into_iter()
        .map(|e| Event::SerializedActEffect { effect: e })
        .collect()
}
