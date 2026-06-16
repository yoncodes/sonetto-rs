use sonettobuf::{ActEffect, effect_type_enum::EffectType, fight_step};

use crate::state::battle::{
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, SkillEmitKind, drain_to_fight_steps},
    manager::buff_mgr::observe_explicit_buff_uid_for_target,
    skill::{
        PhaseFilter, build_skill_act_effect,
        cache::{SKILL_CACHE, resolve_skill_effect_id},
    },
    types::{behavior::BehaviorType, condition::ConditionType},
    utils::buff_has_bloodpool,
};

/// Executes a single passive skill for an entity and syncs state from emitted effects.
/// Returns the collected effects or None if the skill fired nothing.
pub fn execute_skill(
    ctx: &mut FightContext<'_>,
    uid: i64,
    target_uid: i64,
    skill_id: i32,
    phase: &PhaseFilter,
) -> Result<Vec<ActEffect>, anyhow::Error> {
    // build_skill_act_effect already returns top-level ActEffect containers (usually 162/FightStep).
    // Passive flow should preserve that container order/shape for live parity.
    let mut skill_effects = build_skill_act_effect(ctx, uid, target_uid, skill_id, phase)?;
    if skill_effects.is_empty() {
        return Ok(vec![]);
    }
    for effect in &mut skill_effects {
        let Some(step) = effect.fight_step.as_ref() else {
            continue;
        };
        if effect.effect_type != Some(EffectType::Fightstep as i32)
            || step.act_type != Some(fight_step::ActType::Skill as i32)
            || step.act_id != Some(skill_id)
        {
            continue;
        }

        let mut queue = EventQueue::new();
        queue.push(BattleEvent::SkillEmit {
            skill_id,
            from: step.from_id.unwrap_or(uid),
            to: step.to_id.unwrap_or(target_uid),
            children: step
                .act_effect
                .clone()
                .into_iter()
                .map(|effect| BattleEvent::SerializedActEffect { effect })
                .collect(),
            kind: SkillEmitKind::EventTriggered,
        });
        let mut event_ctx = EventContext {
            fight: ctx.fight,
            buff_mgr: &mut ctx.managers.buff_mgr,
            entity_mgr: &mut ctx.managers.entity_mgr,
            bloodtithe: &mut ctx.mechanics.bloodtithe,
        };
        let drained = drain_to_fight_steps(queue.drain(), &mut event_ctx)
            .into_iter()
            .next()
            .expect("event-triggered passive skill wrapper should serialize to one ActEffect");
        if drained.effect_type != Some(EffectType::Fightstep as i32) {
            continue;
        }
        *effect = drained;
    }

    // Keep execute_skill output shape intact: each returned 162 (and any side effects)
    // must remain a sibling entry, not folded into the first 162.
    // Combat callers (trigger/combat.rs, round_mgr.rs sweeps, card_mgr.rs)
    // replay these steps via calculate_mgr::play_step_data, which applies
    // ExPointChange effects itself. Battle-start callers
    // (fight_data_mgr::build_initial_round via run_battle_start) do NOT
    // replay steps — so entity_mgr must be mirrored here for non-combat
    // phases only. Mirroring in combat would double-apply (heroes gained
    // 2x expected moxie on TeammateUseExSkill + AddExPointWithMax).
    let should_mirror_ex = !phase.is_combat();

    let mut effects = Vec::with_capacity(skill_effects.len());
    for mut effect in skill_effects {
        if should_inline_use_ex_replace_buff2(skill_id, phase, &effect) {
            if let Some(step) = effect.fight_step.take() {
                sync_buff_state(ctx, uid, &step.act_effect);
                if should_mirror_ex {
                    sync_ex_point_state(ctx, uid, &step.act_effect);
                }
                effects.extend(step.act_effect);
                continue;
            }
        } else if effect.effect_type == Some(EffectType::Fightstep as i32)
            && let Some(step) = effect.fight_step.as_ref()
        {
            sync_buff_state(ctx, uid, &step.act_effect);
            if should_mirror_ex {
                sync_ex_point_state(ctx, uid, &step.act_effect);
            }
        }
        effects.push(effect);
    }

    let bt_pending = std::mem::take(&mut ctx.mechanics.bloodtithe.pending_effects);
    // Combat callers replay these effects via calculate_mgr::play_step_data
    // (trigger/combat.rs, round_mgr.rs sweeps, card_mgr.rs). Battle-start
    // callers do not replay them, so mirror EX only in non-combat phases to
    // avoid double-application.
    if !phase.is_combat() {
        for effect in &bt_pending {
            if effect.effect_type == Some(EffectType::Expointchange as i32)
                && let Some(target) = effect.target_id
            {
                ctx.managers
                    .entity_mgr
                    .add_ex_point(target, effect.effect_num.unwrap_or(0));
            }
        }
    }
    effects.extend(bt_pending);

    Ok(effects)
}

fn should_inline_use_ex_replace_buff2(
    skill_id: i32,
    phase: &PhaseFilter,
    effect: &ActEffect,
) -> bool {
    let PhaseFilter::Combat(event) = phase else {
        return false;
    };
    if !event.active_use_skill || !event.used_ex_skill {
        return false;
    }

    let Some(step) = effect.fight_step.as_ref() else {
        return false;
    };
    if effect.effect_type != Some(EffectType::Fightstep as i32)
        || step.act_type != Some(fight_step::ActType::Skill as i32)
        || step.act_id != Some(skill_id)
        || step.act_effect.is_empty()
        || step
            .act_effect
            .iter()
            .any(|effect| effect.fight_step.is_some())
        || !step.act_effect.iter().all(|effect| {
            matches!(
                effect.effect_type,
                Some(x)
                    if x == EffectType::Buffdel as i32 || x == EffectType::Buffupdate as i32
            )
        })
    {
        return false;
    }

    let effect_id = resolve_skill_effect_id(skill_id);
    SKILL_CACHE
        .get(&effect_id)
        .map(|rows| {
            rows.iter().any(|row| {
                matches!(row.condition, ConditionType::UseExSkill)
                    && matches!(row.behavior, BehaviorType::ReplaceBuff2 { .. })
            })
        })
        .unwrap_or(false)
}

/// Mirror ExPointChange (type 111) effects into entity_mgr. Only safe to
/// call when the caller does NOT replay the same effects through
/// calculate_mgr::play_step_data (which itself calls add_ex_point). Used for
/// battle-start passives, not combat triggers.
fn sync_ex_point_state(ctx: &mut FightContext<'_>, uid: i64, effects: &[ActEffect]) {
    walk_nested_effects(effects, |effect| {
        if effect.effect_type != Some(EffectType::Expointchange as i32) {
            return;
        }
        let target = effect.target_id.unwrap_or(uid);
        let amount = effect.effect_num.unwrap_or(0);
        ctx.managers.entity_mgr.add_ex_point(target, amount);
    });
}

/// Syncs buff_mgr from BuffAdd effects emitted by a skill.
fn sync_buff_state(ctx: &mut FightContext<'_>, uid: i64, effects: &[ActEffect]) {
    walk_nested_effects(effects, |effect| {
        if effect.effect_type == Some(EffectType::Buffadd as i32) {
            let target_uid = effect.target_id.unwrap_or(uid);
            let Some(buff_id) = effect.effect_num else {
                return;
            };
            let from_uid = effect.buff.as_ref().and_then(|b| b.from_uid).unwrap_or(0);
            let count = effect.buff.as_ref().and_then(|b| b.count).unwrap_or(0);
            let layer = effect.buff.as_ref().and_then(|b| b.layer).unwrap_or(0);
            let buff_uid = effect.buff.as_ref().and_then(|b| b.uid).unwrap_or(0);

            observe_explicit_buff_uid_for_target(target_uid, buff_uid);
            ctx.managers
                .buff_mgr
                .add_with_uid(target_uid, buff_id, from_uid, 0, count, layer, buff_uid);

            if !ctx.mechanics.bloodtithe.initialized && buff_has_bloodpool(buff_id) {
                ctx.mechanics.bloodtithe.initialized = true;
            }
            return;
        }

        if effect.effect_type == Some(EffectType::Buffupdate as i32) {
            let target_uid = effect.target_id.unwrap_or(uid);
            let buff_id = effect.buff.as_ref().and_then(|b| b.buff_id).unwrap_or(0);
            let from_uid = effect.buff.as_ref().and_then(|b| b.from_uid).unwrap_or(0);
            let count = effect.buff.as_ref().and_then(|b| b.count).unwrap_or(0);
            let layer = effect.buff.as_ref().and_then(|b| b.layer).unwrap_or(0);
            let buff_uid = effect.buff.as_ref().and_then(|b| b.uid).unwrap_or(0);
            if buff_id != 0 && buff_uid != 0 {
                observe_explicit_buff_uid_for_target(target_uid, buff_uid);
                ctx.managers
                    .buff_mgr
                    .add_with_uid(target_uid, buff_id, from_uid, 0, count, layer, buff_uid);
            }
        }
    });
}

fn walk_nested_effects<'a>(effects: &'a [ActEffect], mut visit: impl FnMut(&'a ActEffect)) {
    enum Node<'a> {
        Slice(&'a [ActEffect], usize),
        Effect(&'a ActEffect),
    }

    let mut stack = vec![Node::Slice(effects, 0)];
    while let Some(node) = stack.pop() {
        match node {
            Node::Slice(slice, idx) => {
                if idx >= slice.len() {
                    continue;
                }
                let effect = &slice[idx];
                stack.push(Node::Slice(slice, idx + 1));
                stack.push(Node::Effect(effect));
                if let Some(step) = effect.fight_step.as_ref() {
                    stack.push(Node::Slice(&step.act_effect, 0));
                }
            }
            Node::Effect(effect) => visit(effect),
        }
    }
}
