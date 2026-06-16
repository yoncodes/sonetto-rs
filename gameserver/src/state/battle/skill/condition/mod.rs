mod action;
mod bloodtithe;
pub mod eval;
pub mod buff;
mod career;
mod combat;
mod enter_fight;
mod ex_point;
mod life;
pub mod misc;

pub mod parser;
pub mod scope;

use self::action::CONDITION_REGISTRY;

use crate::state::battle::{
    manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr},
    mechanics::bloodtithe::BloodtitheState,
    skill::{
        cache::{SKILL_CACHE, resolve_skill_effect_id},
        phase::TriggerState,
    },
    types::behavior::BehaviorType,
};
use sonettobuf::Fight;
use std::collections::HashSet;

pub use crate::state::battle::types::condition::ConditionType;

#[derive(Clone, Copy)]
pub struct ConditionEval<'a> {
    pub(super) fight: &'a Fight,
    pub(super) buff_mgr: &'a BuffMgr,
    pub(super) entity_mgr: &'a EntityMgr,
    pub(super) bloodtithe: &'a BloodtitheState,
    pub(super) caster_uid: i64,
    pub(super) target_uid: i64,
    pub(super) condition_target: i32,
    pub(super) has_trigger_state: bool,
    pub(super) active_card_cast_uids: Option<&'a HashSet<i64>>,
}

impl<'a> ConditionEval<'a> {
    pub fn new(
        fight: &'a Fight,
        buff_mgr: &'a BuffMgr,
        entity_mgr: &'a EntityMgr,
        bloodtithe: &'a BloodtitheState,
        caster_uid: i64,
    ) -> Self {
        Self {
            fight,
            buff_mgr,
            entity_mgr,
            bloodtithe,
            caster_uid,
            target_uid: caster_uid,
            condition_target: 0,
            has_trigger_state: false,
            active_card_cast_uids: None,
        }
    }

    pub fn for_target(mut self, target_uid: i64) -> Self {
        self.target_uid = target_uid;
        self
    }

    pub fn with_trigger_state(mut self, has_trigger_state: bool) -> Self {
        self.has_trigger_state = has_trigger_state;
        self
    }

    pub fn with_condition_target(mut self, condition_target: i32) -> Self {
        self.condition_target = condition_target;
        self
    }

    pub fn with_active_card_cast_uids(mut self, active_card_cast_uids: &'a HashSet<i64>) -> Self {
        self.active_card_cast_uids = Some(active_card_cast_uids);
        self
    }

    pub fn resolve_entity_target_uid(&self) -> i64 {
        match self.condition_target {
            103 => self.caster_uid,
            // TODO(condition-target): handle MySideAll/EnemySideAll and other non-Self groups.
            _ => self.target_uid,
        }
    }

    pub fn check(&self, condition: &ConditionType) -> bool {
        if matches!(condition, ConditionType::NoActRound) {
            return self.has_trigger_state
                && self
                    .active_card_cast_uids
                    .map(|uids| !uids.contains(&self.caster_uid))
                    .unwrap_or(true);
        }

        if self.has_trigger_state && is_grouped_combat_event_condition(condition) {
            return true;
        }

        if !self.has_trigger_state
            && matches!(
                condition,
                ConditionType::PerDecrExPoint { .. }
                    | ConditionType::UseExSkill
                    | ConditionType::ActiveUseSkill
                    | ConditionType::TeammateUseExSkill
                    | ConditionType::ActiveUseSkillId { .. }
                    | ConditionType::ActOrder { .. }
                    | ConditionType::UseSkillEffectTag { .. }
                    | ConditionType::UseSpecificSkill { .. }
                    | ConditionType::UseHurtSkill
                    | ConditionType::TriggerBullet
                    | ConditionType::BeAttacked
                    | ConditionType::HurtMagic
                    | ConditionType::LostExPoint { .. }
                    | ConditionType::HurtNotRestraint
                    | ConditionType::HurtRestraint
                    | ConditionType::TeammateInjuryCount { .. }
                    | ConditionType::TeammateInjuryCountNotReset { .. }
                    | ConditionType::TeamInjuryCountRound
                    | ConditionType::BuffIdDel { .. }
                    | ConditionType::NoActRound
            )
        {
            return false;
        }

        for cluster in CONDITION_REGISTRY {
            if let Some(result) = cluster.check(condition, self) {
                return result;
            }
        }
        false
    }

    pub fn check_with_random_target(
        &self,
        random_target_uid: i64,
        condition: &ConditionType,
    ) -> bool {
        match condition {
            ConditionType::EnterFightAnd(conds) => conds.iter().all(|cond| {
                let leaf_target = if matches!(cond, ConditionType::Random { .. }) {
                    random_target_uid
                } else {
                    self.target_uid
                };
                self.for_target(leaf_target)
                    .with_trigger_state(true)
                    .check(cond)
            }),
            ConditionType::EnterFightOr(conds) => conds.iter().any(|cond| {
                let leaf_target = if matches!(cond, ConditionType::Random { .. }) {
                    random_target_uid
                } else {
                    self.target_uid
                };
                self.for_target(leaf_target)
                    .with_trigger_state(true)
                    .check(cond)
            }),
            _ => self.with_trigger_state(true).check(condition),
        }
    }
}

/// Evaluate a (possibly composite) condition by applying `leaf(cond)` at each
/// non-composite node. `EnterFightAnd` folds with logical AND, `EnterFightOr`
/// folds with logical OR. Non-composite `cond`s are evaluated directly.
pub fn fold<F>(condition: &ConditionType, leaf: &mut F) -> bool
where
    F: FnMut(&ConditionType) -> bool,
{
    match condition {
        ConditionType::EnterFightAnd(conds) => conds.iter().all(|cond| fold(cond, leaf)),
        ConditionType::EnterFightOr(conds) => conds.iter().any(|cond| fold(cond, leaf)),
        other => leaf(other),
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct CombatEventConditionOptions {
    pub include_per_decr_ex_point: bool,
    pub include_career_check: bool,
    pub include_hurt_magic: bool,
    pub include_lost_ex_point: bool,
}

pub(crate) fn is_combat_event_condition(
    condition: &ConditionType,
    options: CombatEventConditionOptions,
) -> bool {
    fold(condition, &mut |cond| {
        is_combat_event_leaf_condition(cond, options)
    })
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ActiveUseSkillConditionContext {
    pub has_skill_use: bool,
    pub skill_id: i32,
    pub action_order_index: i32,
}

pub(crate) fn eval_active_use_skill_condition(
    condition: &ConditionType,
    context: ActiveUseSkillConditionContext,
) -> Option<bool> {
    match condition {
        ConditionType::ActiveUseSkill => Some(context.has_skill_use),
        ConditionType::ActiveUseSkillId { skill_ids } => {
            Some(context.has_skill_use && skill_ids.contains(&context.skill_id))
        }
        ConditionType::ActOrder { order_index } => Some(
            context.has_skill_use
                && context.action_order_index > 0
                && context.action_order_index == *order_index,
        ),
        ConditionType::UseSkillEffectTag { effect_tag } => Some(
            context.has_skill_use
                && active_skill_effect_tag(context.skill_id)
                    .map(|tag| tag == *effect_tag)
                    .unwrap_or(false),
        ),
        ConditionType::UseSpecificSkill { skill_id } => {
            Some(context.has_skill_use && skill_matches_specific(context.skill_id, *skill_id))
        }
        ConditionType::UseHurtSkill => {
            Some(context.has_skill_use && skill_is_hurt(context.skill_id))
        }
        _ => None,
    }
}

pub(crate) fn is_combat_event_leaf_condition(
    condition: &ConditionType,
    options: CombatEventConditionOptions,
) -> bool {
    match condition {
        ConditionType::ActiveUseSkill
        | ConditionType::ActiveUseSkillId { .. }
        | ConditionType::ActOrder { .. }
        | ConditionType::UseSkillEffectTag { .. }
        | ConditionType::UseSpecificSkill { .. }
        | ConditionType::UseHurtSkill
        | ConditionType::CombatNone
        | ConditionType::UseExSkill
        | ConditionType::TeammateUseExSkill
        | ConditionType::BeAttacked
        | ConditionType::HurtNotRestraint
        | ConditionType::HurtRestraint
        | ConditionType::TeammateInjuryCount { .. }
        | ConditionType::TeammateInjuryCountNotReset { .. }
        | ConditionType::TeamInjuryCountRound
        | ConditionType::NoActRound
        | ConditionType::BuffIdDel { .. }
        | ConditionType::TriggerBullet => true,
        ConditionType::PerDecrExPoint { .. } => options.include_per_decr_ex_point,
        ConditionType::CareerCheck { .. } => options.include_career_check,
        ConditionType::HurtMagic => options.include_hurt_magic,
        ConditionType::LostExPoint { .. } => options.include_lost_ex_point,
        _ => false,
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ReactivePassiveConditionOptions {
    pub include_be_attacked: bool,
    pub include_teammate_injury_count: bool,
}

pub(crate) fn is_reactive_passive_condition(
    condition: &ConditionType,
    options: ReactivePassiveConditionOptions,
) -> bool {
    fold(condition, &mut |cond| {
        is_reactive_passive_leaf_condition(cond, options)
    })
}

pub(crate) fn is_reactive_passive_leaf_condition(
    condition: &ConditionType,
    options: ReactivePassiveConditionOptions,
) -> bool {
    match condition {
        ConditionType::BeAttacked => options.include_be_attacked,
        ConditionType::TeammateInjuryCount { .. }
        | ConditionType::TeammateInjuryCountNotReset { .. } => {
            options.include_teammate_injury_count
        }
        _ => false,
    }
}

pub(crate) fn active_use_skill_context_for_trigger_state(
    event: &TriggerState,
) -> ActiveUseSkillConditionContext {
    ActiveUseSkillConditionContext {
        has_skill_use: event.active_use_skill,
        skill_id: event.skill_id,
        action_order_index: event.action_order_index,
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TriggerStateConditionOptions {
    pub include_none: bool,
    pub include_combat_none: bool,
    pub include_hurt_magic: bool,
    pub include_lost_ex_point: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct TriggerStateConditionContext<'a> {
    pub event: &'a TriggerState,
    pub owner_uid: i64,
}

pub(crate) fn eval_trigger_state_condition(
    condition: &ConditionType,
    context: TriggerStateConditionContext<'_>,
    options: TriggerStateConditionOptions,
) -> Option<bool> {
    if let Some(pass) = eval_active_use_skill_condition(
        condition,
        active_use_skill_context_for_trigger_state(context.event),
    ) {
        return Some(pass);
    }

    match condition {
        ConditionType::None => options.include_none.then_some(true),
        ConditionType::CombatNone => options.include_combat_none.then_some(true),
        ConditionType::UseExSkill => {
            Some(context.event.active_use_skill && context.event.used_ex_skill)
        }
        ConditionType::TeammateUseExSkill => Some(context.event.teammate_use_ex_skill),
        ConditionType::BeAttacked => Some(context.event.be_attacked),
        ConditionType::HurtMagic => options
            .include_hurt_magic
            .then_some(context.event.hurt_magic),
        ConditionType::LostExPoint { .. } => options
            .include_lost_ex_point
            .then_some(context.event.lost_ex_point),
        ConditionType::HurtNotRestraint => Some(context.event.hurt_not_restraint),
        ConditionType::HurtRestraint => Some(context.event.hurt_restraint),
        ConditionType::TeammateInjuryCount { threshold } => {
            Some(context.event.teammate_injury_count >= *threshold)
        }
        ConditionType::TeammateInjuryCountNotReset { threshold } => {
            Some(context.event.teammate_injury_count_not_reset >= *threshold)
        }
        ConditionType::TeamInjuryCountRound => Some(context.event.team_injury_count_round),
        ConditionType::BuffIdDel { buff_ids } => Some(buff::deleted_matches(
            &context.event.deleted_buff_ids,
            buff_ids,
        )),
        ConditionType::TriggerBullet => Some(context.event.trigger_bullet),
        ConditionType::NoActRound => Some(context.event.no_act_round_for(context.owner_uid)),
        _ => None,
    }
}

fn active_skill_effect_tag(skill_id: i32) -> Option<i32> {
    if skill_id <= 0 {
        return None;
    }
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    cfg.skill_effect
        .iter()
        .find(|row| row.id == effect_id)
        .map(|row| row.effect_tag)
}

fn skill_matches_specific(skill_id: i32, wanted: i32) -> bool {
    if skill_id <= 0 || wanted <= 0 {
        return false;
    }
    if skill_id == wanted || resolve_skill_effect_id(skill_id) == wanted {
        return true;
    }

    let cfg = config::configs::get();
    let Some(skill) = cfg.skill.get(skill_id) else {
        return false;
    };

    if wanted <= 3 && skill.skill_rank == wanted {
        return true;
    }

    wanted == 4
}

fn skill_is_hurt(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    if cfg
        .skill_effect
        .iter()
        .find(|row| row.id == effect_id)
        .map(|row| row.damage_rate > 0)
        .unwrap_or(false)
    {
        return true;
    }

    SKILL_CACHE
        .get(&effect_id)
        .map(|rows| {
            rows.iter()
                .any(|row| matches!(row.behavior, BehaviorType::Damage { .. }))
        })
        .unwrap_or(false)
}

/// Compatibility shim for condition walkers that still pass the full context piecemeal.
#[allow(clippy::too_many_arguments)]
pub fn check_condition(
    fight: &Fight,
    buff_mgr: &BuffMgr,
    entity_mgr: &EntityMgr,
    bloodtithe: &BloodtitheState,
    caster_uid: i64,
    target_uid: i64,
    has_trigger_state: bool,
    condition: &ConditionType,
) -> bool {
    ConditionEval::new(fight, buff_mgr, entity_mgr, bloodtithe, caster_uid)
        .for_target(target_uid)
        .with_trigger_state(has_trigger_state)
        .check(condition)
}

fn is_grouped_combat_event_condition(condition: &ConditionType) -> bool {
    is_combat_event_leaf_condition(
        condition,
        CombatEventConditionOptions {
            include_hurt_magic: true,
            include_lost_ex_point: true,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonettobuf::{FightEntityInfo, FightTeam};

    fn build_fight() -> Fight {
        Fight {
            attacker: Some(FightTeam {
                entitys: vec![FightEntityInfo {
                    uid: Some(1),
                    career: Some(1),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            defender: Some(FightTeam {
                entitys: vec![FightEntityInfo {
                    uid: Some(-1),
                    career: Some(4),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn assert_false_without_trigger_state(condition: ConditionType) {
        let mut entity_mgr = EntityMgr::default();
        entity_mgr.set_recent_decr_ex_point(1, 3);
        let fight = build_fight();
        let buff_mgr = BuffMgr::new();
        let bloodtithe = BloodtitheState::new();

        assert!(!check_condition(
            &fight,
            &buff_mgr,
            &entity_mgr,
            &bloodtithe,
            1,
            -1,
            false,
            &condition,
        ));
    }

    #[test]
    fn per_decr_ex_point_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::PerDecrExPoint { threshold: 1 });
    }

    #[test]
    fn use_ex_skill_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::UseExSkill);
    }

    #[test]
    fn teammate_use_ex_skill_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::TeammateUseExSkill);
    }

    #[test]
    fn active_use_skill_id_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::ActiveUseSkillId {
            skill_ids: vec![1234],
        });
    }

    #[test]
    fn be_attacked_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::BeAttacked);
    }

    #[test]
    fn hurt_not_restraint_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::HurtNotRestraint);
    }

    #[test]
    fn hurt_restraint_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::HurtRestraint);
    }

    #[test]
    fn teammate_injury_count_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::TeammateInjuryCount { threshold: 1 });
    }

    #[test]
    fn team_injury_count_round_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::TeamInjuryCountRound);
    }

    #[test]
    fn buff_id_del_is_false_without_trigger_state() {
        assert_false_without_trigger_state(ConditionType::BuffIdDel {
            buff_ids: vec![4150003],
        });
    }

    #[test]
    fn trigger_bullet_is_true_with_trigger_state() {
        let entity_mgr = EntityMgr::default();
        let fight = build_fight();
        let buff_mgr = BuffMgr::new();
        let bloodtithe = BloodtitheState::new();

        assert!(check_condition(
            &fight,
            &buff_mgr,
            &entity_mgr,
            &bloodtithe,
            1,
            -1,
            true,
            &ConditionType::TriggerBullet,
        ));
    }
}
