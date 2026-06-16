mod be_attacked;
mod buff_id_add;
mod buff_id_del;
mod dead;
mod enter_fight;
mod has_buff_id;
mod hero_round_interval;
mod none;
mod no_buff_id;
mod per_buff_id_count;
mod teammate_dead;
mod use_ex_skill;
mod use_skill_effect_tag;
mod r#type;

use super::condition_eval::ConditionEval;
use r#type::{ConditionType, condition_type};
use super::target::Target;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hook {
    Dead,
    EnterFight,
    RoundStart,
    RoundEnd,
    BattleStart,
    UseCard,
    MoveCard,
    CardUpgrade,
    BuffAdd,
    BuffLost,
    EvalActiveSkill,
    EvalBeingAttacked,
    UseExSkill,
    AfterAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionOp {
    And,
    Or,
}

#[derive(Debug, Clone)]
pub struct Condition {
    pub hooks: &'static [Hook],
    pub cond_type: ConditionType,
    pub target: Target,
    pub params: Vec<i32>,
}

impl Condition {
    pub fn check(&self, owner_uid: i64, eval: ConditionEval<'_>) -> Option<i32> {
        eval_condition(self.cond_type, self.target, &self.params, owner_uid, eval)
    }
}

fn eval_condition(cond_type: ConditionType, target: Target, params: &[i32], owner_uid: i64, eval: ConditionEval<'_>) -> Option<i32> {
    match cond_type { ConditionType::_101None | ConditionType::_102None | ConditionType::_104None | ConditionType::_201None | ConditionType::_203None | ConditionType::_208None | ConditionType::_210None | ConditionType::_304None | ConditionType::_301None => if none::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_45104HeroRoundInterval => if hero_round_interval::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_22209BeAttacked => if be_attacked::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_8Dead => if dead::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_812Dead => if dead::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_17TeammateDead => if teammate_dead::check(owner_uid, eval) { Some(1) } else { None },
        ConditionType::_5EnterFight | ConditionType::_5021EnterFight => if enter_fight::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_25210UseExSkill => if use_ex_skill::check(target, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_49BuffIdDel => if buff_id_del::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_10BuffIdAdd => if buff_id_add::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_19201HasBuffId
        | ConditionType::_19208HasBuffId
        | ConditionType::_19202HasBuffId
        | ConditionType::_19209HasBuffId
        | ConditionType::_19210HasBuffId
        | ConditionType::_19203HasBuffId => if has_buff_id::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_57208NoBuffId => if no_buff_id::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_34210UseSkillEffectTag => if use_skill_effect_tag::check(target, params, owner_uid, eval) { Some(1) } else { None },
        ConditionType::_61003PerBuffIdCount
        | ConditionType::_61203PerBuffIdCount => per_buff_id_count::check(target, params, owner_uid, eval),
        other => {
            tracing::warn!("unimplemented condition type: {:?}", other);
            None
        }
    }
}

fn hooks_for_type(cond_type: ConditionType, cond_target: i32) -> &'static [Hook] {
    match cond_type {
        ConditionType::_45104HeroRoundInterval => &[hero_round_interval::HOOK],
        ConditionType::_101None | ConditionType::_102None | ConditionType::_104None => &[Hook::RoundStart],
        ConditionType::_201None | ConditionType::_203None => &[Hook::EvalActiveSkill],
        ConditionType::_208None | ConditionType::_210None => &[Hook::AfterAction],
        ConditionType::_304None | ConditionType::_301None => &[Hook::RoundEnd],
        ConditionType::_22209BeAttacked => &[be_attacked::HOOK],
        ConditionType::_8Dead => &[dead::HOOK],
        ConditionType::_812Dead => &[dead::HOOK],
        ConditionType::_17TeammateDead => &[teammate_dead::HOOK],
        ConditionType::_5EnterFight | ConditionType::_5021EnterFight => &[enter_fight::HOOK],
        ConditionType::_19201HasBuffId | ConditionType::_19208HasBuffId | ConditionType::_19203HasBuffId => &[Hook::EvalActiveSkill],
        ConditionType::_25210UseExSkill => &[Hook::UseExSkill],
        ConditionType::_49BuffIdDel => &[buff_id_del::HOOK],
        ConditionType::_10BuffIdAdd => &[buff_id_add::HOOK],
        ConditionType::_19202HasBuffId | ConditionType::_19209HasBuffId => &[Hook::EvalBeingAttacked],
        ConditionType::_19210HasBuffId => &[Hook::AfterAction],
        ConditionType::_61003PerBuffIdCount
        | ConditionType::_61203PerBuffIdCount  => &[Hook::EvalActiveSkill],
        ConditionType::_57208NoBuffId => &[no_buff_id::HOOK],
        ConditionType::_34210UseSkillEffectTag => &[use_skill_effect_tag::HOOK],
       _ => &[],
    }
}

/// Parses a condition raw string into individual `Condition`s and the combining op.
/// Returns `None` if no known condition types are found.
pub fn parse(raw: &str, cond_target: i32, _owner_uid: i64) -> Option<(Vec<Condition>, ConditionOp)> {
    if raw.is_empty() { return None; }
    let op = if raw.contains('&') { ConditionOp::And } else { ConditionOp::Or };
    let sep = if op == ConditionOp::And { '&' } else { '|' };
    let target = Target::from_id(cond_target);
    let conditions: Vec<Condition> = raw.split(sep)
        .filter_map(|seg| {
            let mut parts = seg.split('#');
            let id: i32 = parts.next()?.parse().ok()?;
            let params: Vec<i32> = parts
                .filter_map(|p| p.trim_end_matches('!').parse().ok())
                .collect();
        let cond_type = condition_type(id)?;
        Some(Condition {
            hooks: hooks_for_type(cond_type, cond_target),
            cond_type,
            target,
                params,
            })
        })
        .collect();
    if conditions.is_empty() { return None; }
    Some((conditions, op))
}
