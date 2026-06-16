use sonettobuf::{BeginRoundOper, FightStep};

use crate::state::battle::{
    BehaviorType,
    context::FightContext,
    fight_step::{ActEffectBuilder, FightStepBuilder},
    round::RoundState,
    skill::cache::{SKILL_CACHE, resolve_skill_effect_id},
    types::ex_point::ExPointType,
};

pub(crate) fn pre_operation_ex_gain(
    ctx: &mut FightContext<'_>,
    state: &RoundState,
    oper: &BeginRoundOper,
) -> Option<FightStep> {
    let op_type = oper.oper_type.unwrap_or(0);
    let to_id = oper.to_id.unwrap_or(0);
    let is_play_like = op_type == 2 || (op_type == 1 && to_id != 0);
    if !is_play_like {
        return None;
    }
    if to_id > 0 {
        return None;
    }

    let card_index = oper.param1.unwrap_or(1).saturating_sub(1) as usize;
    let card = state.selected_cards.get(card_index)?;
    let caster_uid = card.uid.unwrap_or(0);
    if caster_uid <= 0 || card.temp_card.unwrap_or(false) {
        return None;
    }

    let ex_type = ctx
        .fight
        .attacker
        .as_ref()
        .and_then(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .find(|e| e.uid == Some(caster_uid))
        })
        .and_then(|e| e.ex_point_type);

    let gains_standard = ex_type
        .and_then(ExPointType::from_i32)
        .map(|t| t == ExPointType::Common)
        .unwrap_or(false);
    if !gains_standard {
        return None;
    }
    let is_direct_ex_card = ctx
        .fight
        .attacker
        .as_ref()
        .and_then(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .find(|e| e.uid == Some(caster_uid))
        })
        .and_then(|e| e.ex_skill)
        .map(|ex_skill| ex_skill == card.skill_id.unwrap_or(0))
        .unwrap_or(false);
    if is_direct_ex_card {
        return None;
    }

    ctx.managers.entity_mgr.add_ex_point(caster_uid, 1);
    Some(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::ex_point_change(caster_uid, 1))
            .build(),
    )
}

pub(crate) fn skill_suppresses_pre_operation_ex(skill_id: i32) -> bool {
    let skill_effect_id = resolve_skill_effect_id(skill_id);
    let Some(rows) = SKILL_CACHE.get(&skill_effect_id) else {
        return false;
    };
    let has_lost_life = rows
        .iter()
        .any(|row| matches!(row.behavior, BehaviorType::LostLife { .. }));
    let has_raspberry_add = rows
        .iter()
        .any(|row| matches!(row.behavior, BehaviorType::RaspberryAddCount { .. }));
    has_lost_life && has_raspberry_add
}

pub(crate) fn standard_action_ex_gain_for_uid(
    ctx: &mut FightContext<'_>,
    caster_uid: i64,
) -> Option<FightStep> {
    let ex_type = [ctx.fight.attacker.as_ref(), ctx.fight.defender.as_ref()]
        .into_iter()
        .flatten()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .find(|e| e.uid == Some(caster_uid))
        .and_then(|e| e.ex_point_type);

    let gains_standard = ex_type
        .and_then(ExPointType::from_i32)
        .map(|t| t == ExPointType::Common)
        .unwrap_or(false);
    if !gains_standard {
        return None;
    }

    ctx.managers.entity_mgr.add_ex_point(caster_uid, 1);
    Some(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::ex_point_change(caster_uid, 1))
            .build(),
    )
}
