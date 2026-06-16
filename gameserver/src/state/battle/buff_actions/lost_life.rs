use sonettobuf::{ActEffect, fight_hurt_info::DamageFromType};
use std::collections::HashMap;

use crate::state::battle::{
    fight_step::ActEffectBuilder,
    manager::buff_mgr::StackConsumeResult,
    types::attr::AttrId,
    utils::{
        career_damage_multiplier, check_career_restraint, get_attr_replace_damage,
        get_exclude_buff_effects,
    },
};

use super::EffectContext;

/// Skill behavior: Damage — standard damage from caster ATK at a given rate.
pub fn apply(
    ctx: &mut EffectContext,
    _pending_attr: Option<&HashMap<(i64, i32), i32>>,
    rate: i32,
    skill_id: i32,
) -> Vec<ActEffect> {
    let mut effects = crate::state::battle::skill::damage::calculate_damage(
        ctx.fight(),
        ctx.buff_mgr(),
        ctx.entity_mgr(),
        ctx.caster_uid(),
        ctx.target_uid(),
        rate,
        skill_id,
        false,
    );

    let did_damage = effects.iter().any(|e| {
        matches!(
            e.effect_type,
            Some(x) if x == sonettobuf::effect_type_enum::EffectType::Damage as i32
                || x == sonettobuf::effect_type_enum::EffectType::Crit as i32
        ) && e.effect_num.unwrap_or(0) > 0
    });

    let target_uid = ctx.target_uid();
    if did_damage && let Some(result) = ctx.buff_mgr_mut().consume_one_stacked_drop_dmg(target_uid)
    {
        match result {
            StackConsumeResult::Updated(buff) => {
                effects.push(
                    crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                        target_uid,
                        buff.from_uid,
                        buff.buff_id,
                        buff.uid,
                        buff.stacks,
                        buff.layer,
                    ),
                );
            }
            StackConsumeResult::Removed(buff) => {
                effects.push(
                    crate::state::battle::fight_step::ActEffectBuilder::buff_del(
                        target_uid,
                        buff.uid,
                        buff.buff_id,
                        buff.from_uid,
                    ),
                );
            }
        }
    }

    effects
}

/// Skill behavior: LostAllLifeByAttr — damage scaled from caster and target attribute values.
/// attr IDs: 100=CurrentHp, 101=MaxHp, 102=Attack, 103=Defense, etc.
/// amounts are permille (‰) of the attribute value.
pub fn lost_all_life_by_attr(
    ctx: &mut EffectContext,
    caster_attr: i32,
    caster_amount: i32,
    target_attr: i32,
    target_amount: i32,
    skill_id: i32,
) -> Vec<ActEffect> {
    let Some(caster) = ctx.caster_entity() else {
        return vec![];
    };
    let Some(target) = ctx.target_entity() else {
        return vec![];
    };

    let get_val = |entity: &sonettobuf::FightEntityInfo, attr_id: i32| -> i32 {
        let attr = entity.attr.as_ref();
        match AttrId::from(attr_id) {
            Some(AttrId::LostHp) => {
                let max = attr.and_then(|a| a.hp).unwrap_or(0);
                let cur = entity.current_hp.unwrap_or(0);
                (max - cur).max(0)
            }
            Some(AttrId::CurrentHp) => entity.current_hp.unwrap_or(0),
            Some(AttrId::Hp) => attr.and_then(|a| a.hp).unwrap_or(0),
            Some(AttrId::Attack) => attr.and_then(|a| a.attack).unwrap_or(0),
            Some(AttrId::Defense) => attr.and_then(|a| a.defense).unwrap_or(0),
            _ => 0,
        }
    };

    let caster_val = get_val(caster, caster_attr);
    let target_val = get_val(target, target_attr);
    let caster_career = caster.career.unwrap_or(0);
    let target_career = target.career.unwrap_or(0);
    let restraint = career_damage_multiplier(caster_career, target_career) as f32 / 1000.0;
    let dmg = ((caster_val as f32 * caster_amount as f32 / 1000.0)
        + (target_val as f32 * target_amount as f32 / 1000.0))
        * restraint;
    let dmg = (dmg as i32).max(1);
    let hurt_effect = if check_career_restraint(caster_career, target_career) {
        2
    } else {
        0
    };

    vec![
        ActEffectBuilder::damage_default(ctx.target_uid(), dmg),
        ActEffectBuilder::hurt_detail_skill(
            ctx.target_uid(),
            dmg,
            skill_id,
            DamageFromType::SkillEffect as i32,
            hurt_effect,
        ),
    ]
}

/// Skill behavior: DamageRealLostLife — real damage then apply a buff to target.
pub fn damage_real_lost_life(
    ctx: &mut EffectContext,
    buff_id: i32,
    rate: i32,
    skill_id: i32,
) -> Vec<ActEffect> {
    let Some(caster) = ctx.caster_entity() else {
        return vec![];
    };
    let Some(target_entity) = ctx.target_entity() else {
        return vec![];
    };

    let effective_atk = if let Some((source_attr, replace_attr, permille)) =
        get_attr_replace_damage(ctx.buff_mgr(), ctx.caster_uid())
    {
        if AttrId::from(source_attr) == Some(AttrId::Attack) {
            let replace_val = match AttrId::from(replace_attr) {
                Some(AttrId::Hp) => caster.attr.as_ref().and_then(|a| a.hp).unwrap_or(0),
                Some(AttrId::CurrentHp) => caster.current_hp.unwrap_or(0),
                Some(AttrId::Attack) => caster.attr.as_ref().and_then(|a| a.attack).unwrap_or(0),
                _ => caster.attr.as_ref().and_then(|a| a.attack).unwrap_or(0),
            };
            replace_val * permille / 1000
        } else {
            caster.attr.as_ref().and_then(|a| a.attack).unwrap_or(0)
        }
    } else {
        caster.attr.as_ref().and_then(|a| a.attack).unwrap_or(0)
    };

    let caster_career = caster.career.unwrap_or(0);
    let target_career = target_entity.career.unwrap_or(0);
    let restraint = career_damage_multiplier(caster_career, target_career) as f32 / 1000.0;
    let dmg = ((effective_atk as f32 * rate as f32 / 1000.0) * restraint) as i32;
    let dmg = dmg.max(1);
    let hurt_effect = if check_career_restraint(caster_career, target_career) {
        2
    } else {
        0
    };

    let mut effects = vec![
        ActEffectBuilder::damage_default(ctx.target_uid(), dmg),
        ActEffectBuilder::hurt_detail_skill(
            ctx.target_uid(),
            dmg,
            skill_id,
            DamageFromType::SkillEffect as i32,
            hurt_effect,
        ),
    ];

    effects.extend(get_exclude_buff_effects(
        ctx.buff_mgr(),
        ctx.target_uid(),
        buff_id,
    ));
    effects
}
