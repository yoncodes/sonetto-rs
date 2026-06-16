pub mod apply;

use sonettobuf::{ActEffect, BeginRoundOper, CardInfo, FightHurtInfo as HurtInfo};
use crate::state::battle::{event_queue::SkillEmitKind, fight_step::ActEffectBuilder};

#[derive(Debug, Clone)]
pub enum Event {
    BuffApply {
        target: i64, buff_id: i32, count: i32, layer: i32,
        from: i64, from_skill_id: i32, config_effect: Option<i32>,
    },
    BuffUpdate { target: i64, buff_uid: i64, new_count: i32, new_layer: i32, buff_id: i32, from_uid: i64 },
    BuffRemove { target: i64, buff_uid: i64, buff_id: i32, from_uid: i64 },
    BuffSyncAddWithUid {
        target: i64, buff_id: i32, from: i64, from_skill_id: i32,
        count: i32, layer: i32, buff_uid: i64,
    },
    BuffSyncAddWithUidAndEmitUpdate {
        target: i64, buff_id: i32, from: i64, from_skill_id: i32,
        sync_count: i32, sync_layer: i32, buff_uid: i64,
        emit_count: i32, emit_layer: i32,
    },
    Damage {
        target: i64, amount: i32, is_crit: bool,
        hurt_info: HurtInfo, from: i64, skill_id: Option<i32>,
    },
    Heal     { target: i64, amount: i32, from: i64 },
    HealCrit { target: i64, amount: i32, from: i64 },
    ExPointChange { target: i64, delta: i32, emit_step: bool },
    PowerChange   { delta: i32 },
    BloodpoolValueChange { team_type: i32, target: i64, delta: i32 },
    BloodpoolMaxChange   { team_type: i32, max: i32 },
    SerializedActEffect  { effect: ActEffect },
    SkillEmit {
        skill_id: i32, from: i64, to: i64,
        children: Vec<Event>, kind: SkillEmitKind,
    },
    EffectMarker { effect_type: i32 },
    CardPlayed           { card: CardInfo, oper: BeginRoundOper },
    CardMoved            { card: CardInfo },
    CardUpgrade          { card: CardInfo },
    SimulateDissolveCard { oper: BeginRoundOper },
    EnterBattle          { entity_uid: i64 },
    Dead                 { entity_uid: i64 },
    RemoveEntityCards    { entity_uid: i64 },
    BuffAdd              { target_uid: i64, buff_id: i32 },
    RemoveBuff           { target_uid: i64, buff_id: i32 },
    AddActionPoint       { entity_uid: i64, delta: i32 },
}

pub fn events_to_act_effects(events: Vec<Event>) -> Vec<ActEffect> {
    events.into_iter().flat_map(event_to_act_effects).collect()
}

fn event_to_act_effects(e: Event) -> Vec<ActEffect> {
    use sonettobuf::effect_type_enum::EffectType;
    use crate::state::battle::fight_step::{make_skill_step, wrap_step};

    match e {
        Event::Dead { entity_uid } =>
            vec![ActEffectBuilder::dead(entity_uid)],

        Event::RemoveEntityCards { entity_uid } =>
            vec![ActEffectBuilder::remove_entity_cards(entity_uid, Some(1))],

        Event::Damage { target, amount, is_crit, mut hurt_info, from, skill_id } => {
            let damage = amount.max(0);
            if hurt_info.from_uid.is_none() { hurt_info.from_uid = Some(from); }
            if hurt_info.skill_id.is_none() {
                if let Some(sid) = skill_id { hurt_info.skill_id = Some(sid); }
            }
            if hurt_info.damage.is_none() { hurt_info.damage = Some(damage); }
            let et = if is_crit { EffectType::Crit as i32 } else { EffectType::Damage as i32 };
            if hurt_info.hurt_effect.is_none() { hurt_info.hurt_effect = Some(et); }
            vec![if is_crit {
                ActEffectBuilder::crit_with_hurt(target, damage, hurt_info.config_effect, hurt_info)
            } else {
                ActEffectBuilder::damage_with_hurt(target, damage, hurt_info.config_effect, hurt_info)
            }]
        }

        Event::Heal { target, amount, .. } =>
            vec![ActEffectBuilder::heal(target, amount, None)],

        Event::HealCrit { target, amount, .. } =>
            vec![ActEffectBuilder::heal_crit(target, amount)],

        Event::BuffApply { target, buff_id, count, layer, from, from_skill_id: _, config_effect } => {
            use crate::state::battle::{
                manager::buff_mgr::next_buff_uid_for_target,
                types::buff::BuffLayerType,
                utils::buff_get_act_common_params,
            };
            let buff_uid = next_buff_uid_for_target(target);
            let act_common_params = buff_get_act_common_params(buff_id);
            vec![ActEffectBuilder::buff_add_with_snapshot(
                target, from, buff_id, buff_uid, 0, count, act_common_params,
                layer, BuffLayerType::Normal as i32, config_effect,
            )]
        }

        Event::BuffUpdate { target, buff_uid, new_count, new_layer, buff_id, from_uid } =>
            vec![ActEffectBuilder::buff_update(target, from_uid, buff_id, buff_uid, new_count, new_layer)],

        Event::BuffRemove { target, buff_uid, buff_id, from_uid } =>
            vec![ActEffectBuilder::buff_del(target, buff_uid, buff_id, from_uid)],

        Event::BuffSyncAddWithUid { .. } => vec![],

        Event::BuffSyncAddWithUidAndEmitUpdate {
            target, buff_id, from, buff_uid, emit_count, emit_layer, ..
        } =>
            vec![ActEffectBuilder::buff_update(target, from, buff_id, buff_uid, emit_count, emit_layer)],

        Event::BloodpoolValueChange { team_type, target, delta } =>
            vec![ActEffectBuilder::bloodpool_value_change(target, team_type, delta)],

        Event::BloodpoolMaxChange { team_type, max } =>
            vec![ActEffectBuilder::bloodpool_max_change(team_type, max)],

        Event::SerializedActEffect { effect } => vec![effect],

        Event::ExPointChange { target, delta, emit_step: true } =>
            vec![ActEffectBuilder::ex_point_change(target, delta)],

        Event::ExPointChange { emit_step: false, .. } => vec![],

        Event::PowerChange { delta } =>
            vec![ActEffectBuilder::power_change(None, delta, None)],

        Event::SkillEmit { skill_id, from, to, children, kind } => {
            let child_effects: Vec<ActEffect> = children.into_iter().flat_map(event_to_act_effects).collect();
            let inner = make_skill_step(from, to, skill_id, 0, child_effects);
            match kind {
                SkillEmitKind::PlayerInitiated | SkillEmitKind::AutomaticPhase | SkillEmitKind::EventTriggered =>
                    vec![wrap_step(inner)],
                SkillEmitKind::EquipmentEmbedded =>
                    vec![wrap_step(inner)],
            }
        }

        Event::EffectMarker { .. }
        | Event::CardPlayed { .. }
        | Event::CardMoved { .. }
        | Event::CardUpgrade { .. }
        | Event::SimulateDissolveCard { .. }
        | Event::EnterBattle { .. }
        | Event::BuffAdd { .. }
        | Event::RemoveBuff { .. }
        | Event::AddActionPoint { .. } => vec![],
    }
}
