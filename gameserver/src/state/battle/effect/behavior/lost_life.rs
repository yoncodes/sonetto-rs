use rand::rngs::StdRng;
use sonettobuf::{Fight, effect_type_enum::EffectType};

use crate::state::battle::{
    buff_actions::{EffectContext, lost_life as lost_life_handler},
    event::Event,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    manager::{buff_mgr::BuffMgr as EventBuffMgr, fight_data_mgr::Managers},
    mechanics::{Mechanics, bloodtithe::BloodtitheState},
    skill::{SkillExecutor, buff},
    types::effects::EffectType as LocalEffectType,
};
use crate::state::battle::effect::behavior::r#type::BehaviourType;
use super::bloodtithe;

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_lost_life(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    caster_uid: i64,
    target: i64,
    mode: i32,
    attr_id: i32,
    permille: i32,
    behavior_id: i32,
    skill_id: i32,
    is_combat: bool,
) -> Vec<sonettobuf::ActEffect> {
    let floor_permille = buff::ban_lost_life_floor_permille(fight, managers, target);
    let mut effects = bloodtithe::lost_life(
        fight,
        &managers.buff_mgr,
        &mut mechanics.bloodtithe,
        caster_uid,
        target,
        mode,
        attr_id,
        permille,
        behavior_id,
        skill_id,
        floor_permille,
    );
    let mut queue_routed_damage = 0;
    if let Some((index, amount, target_id, hurt_info)) =
        effects.iter().enumerate().find_map(|(index, effect)| {
            (effect.effect_type == Some(EffectType::Damage as i32)
                && effect.effect_num.unwrap_or(0) > 0
                && effect.target_id.is_some()
                && effect.hurt_info.is_some())
            .then_some((
                index,
                effect.effect_num.unwrap_or(0),
                effect.target_id.unwrap_or(0),
                effect.hurt_info.clone().unwrap_or_default(),
            ))
        })
    {
        let mut queue = EventQueue::new();
        queue.push(BattleEvent::Damage {
            target: target_id,
            amount,
            is_crit: false,
            hurt_info,
            from: caster_uid,
            skill_id: Some(skill_id),
        });
        let mut synthetic_fight = Fight::default();
        let mut synthetic_buff_mgr = EventBuffMgr::new();
        let mut synthetic_bloodtithe = BloodtitheState::new();
        let mut event_ctx = EventContext {
            fight: &mut synthetic_fight,
            buff_mgr: &mut synthetic_buff_mgr,
            entity_mgr: &mut managers.entity_mgr,
            bloodtithe: &mut synthetic_bloodtithe,
        };
        if let Some(routed) = drain_to_fight_steps(queue.drain(), &mut event_ctx)
            .into_iter()
            .next()
        {
            effects[index] = routed;
            queue_routed_damage = amount;
        }
    }
    let damage = effects
        .iter()
        .find(|e| {
            matches!(
                e.effect_type,
                Some(t)
                    if t == EffectType::Damage as i32
                        || t == EffectType::Crit as i32
                        || t == LocalEffectType::OriginDamage as i32
                        || t == LocalEffectType::OriginCrit as i32
            )
        })
        .and_then(|e| e.effect_num)
        .unwrap_or(0);
    if queue_routed_damage > 0 {
        mechanics.shadow_cloak.add(target, queue_routed_damage);
    } else if damage > 0 {
        managers.entity_mgr.apply_damage(target, damage);
        mechanics.shadow_cloak.add(target, damage);
    }
    if !is_combat {
        for e in &effects {
            if e.effect_type == Some(111)
                && let Some(uid) = e.target_id
            {
                managers.entity_mgr.add_ex_point(uid, e.effect_num.unwrap_or(0));
            }
        }
    }
    effects
}

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, skill_id: i32, raw: &str, _count: i32, beh_type: BehaviourType,
) -> Vec<Event> {
    let mut out = Vec::new();
    for target in targets {
        let effects = match beh_type {
            BehaviourType::_30005LostLife
            | BehaviourType::_30006LostLife
            | BehaviourType::_30010LostLifeNotFixed
            | BehaviourType::_30018LostLife
            | BehaviourType::_60226LostLife2 => {
                let parts: Vec<&str> = raw.split('#').collect();
                let mode: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let attr_id: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
                let permille: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                let behavior_id: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
                apply_lost_life(fight, managers, mechanics, entity_uid, target, mode, attr_id, permille, behavior_id, skill_id, true)
            }
            BehaviourType::_60216DamageRealLostLife => {
                let parts: Vec<&str> = raw.split('#').collect();
                let buff_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let rate: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
                let mut effect_ctx = EffectContext::new(fight, managers, mechanics, entity_uid, target);
                lost_life_handler::damage_real_lost_life(&mut effect_ctx, buff_id, rate, skill_id)
            }
            BehaviourType::_60213SurvivalHealth | BehaviourType::_60146OriginDamageByTeamAttr => {
                tracing::warn!("unimplemented lost_life behaviour: {:?}", beh_type);
                vec![]
            }
            _ => vec![],
        };
        out.extend(effects.into_iter().map(|e| Event::SerializedActEffect { effect: e }));
    }
    out
}
