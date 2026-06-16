//! Round-end emission helpers split out of the `FightRoundMgr`
//! god-class.
//!
//! These functions assemble the FightStep stream that closes a round:
//! the bloodtithe transition steps, the `effect_type=276` ChangeRound
//! marker, the terminal attacker passive, and the round-end broadcast
//! (with a battle-rule-derived synthetic `530000112` fallback for
//! battles whose `addition_rule` references the boss state cycle
//! `530000151`).
//!
//! `FightRoundMgr` is a unit struct, so passing `mgr: &FightRoundMgr`
//! is a namespacing convention rather than threading state. Callers
//! still get to use the existing `apply_step_and_maybe_sync` and
//! `collect_battle_rule_skills` methods through the reference;
//! consolidating both in a free-function module shrinks the
//! `round_mgr.rs` god-class.

use std::collections::HashMap;

use anyhow::Result;
use sonettobuf::{ActEffect, CardInfo, FightStep, effect_type_enum::EffectType, fight_step};

use crate::state::battle::{
    context::FightContext,
    fight_step::{ActEffectBuilder, FightStepBuilder, wrap_step},
    heroes::rubuska,
    manager::{
        buff_mgr::next_buff_uid_for_target,
        round_mgr::{apply_step_and_maybe_sync, skill_has_no_act_round_condition},
        traits::Manager,
    },
    mechanics::bloodtithe,
    passives::{
        collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill,
    },
    round::step_shape::build_effect_step,
    rule::collect::collect_battle_rule_skills,
    step_walker,
    steps::broadcast,
};

const BOSS_STATE_CYCLE_SKILL_ID: i32 = 530000151;
const BOSS_STATE_ACTIVE_BUFF_ID: i32 = 530000111;
const BOSS_STATE_INACTIVE_BUFF_ID: i32 = 530000112;
const ROUND_END_STATE_MARKER_SKILL_ID: i32 = 4150001;
const SMALL_ROUND_END_EFFECT_TYPE: i32 = 211;
const CHANGE_ROUND_BUNDLE_MARKER: i32 = 212;
const SOTHEBY_LATE_GRANT_SKILL_ID: i32 = 30090146;
const WILLOW_LATE_FRAGMENT_SKILL_ID: i32 = 31040141;

#[derive(Clone)]
struct BossStateSnapshot {
    buff_uid: i64,
    from_uid: i64,
    count: i32,
    layer: i32,
    duration: i32,
}

/// Emit the canonical terminal-round step stream:
/// 1. Bloodtithe round-transition steps (one per active bloodpool).
/// 2. The `effect_type=276` ChangeRound marker carrying
///    `selected_for_round_end`.
/// 3. The terminal attacker round-end passive (if one fires).
/// 4. The round-end broadcast (with synthetic `530000112` fallback
///    for `addition_rule`-derived `530000151` battles).
pub(crate) fn emit_terminal_round_steps(
    ctx: &mut FightContext<'_>,
    selected_for_round_end: Vec<CardInfo>,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    for step in bloodtithe::build_round_transition_bloodtithe_steps(ctx, collected) {
        apply_step_and_maybe_sync(ctx, &step, true)?;
        steps.push(step);
    }

    steps.push(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::allocate_card_energy(
                selected_for_round_end,
            ))
            .build(),
    );
    if let Some(raw_step) = build_terminal_attacker_round_end_passive_step(ctx, collected) {
        apply_step_and_maybe_sync(ctx, &raw_step, true)?;
        steps.push(build_effect_step(vec![wrap_step(raw_step)]));
    }

    let broadcast = collect_terminal_round_end_broadcast(ctx);
    if !broadcast.is_empty() {
        steps.push(build_effect_step(broadcast));
    }

    Ok(())
}

/// Snapshot one duration tick of buff state, walk the broadcast
/// collector, then restore. Used by both the per-round and terminal
/// round-end paths so the broadcast reflects the post-tick state
/// without committing the tick to live state.
///
/// TODO(event-queue): same snapshot/restore preview pattern as the
/// defender-side block at the call site. Migrate to a
/// `PreviewRoundEndTick` event when EventQueue Phase 5 covers
/// preview semantics.
pub(crate) fn collect_attacker_round_end_broadcast(
    ctx: &mut FightContext<'_>,
    injected_channel_buffs: bool,
    preview_round_end_tick: bool,
) -> Vec<ActEffect> {
    let mut broadcast = if preview_round_end_tick || ctx.fight.cur_round.unwrap_or(1) == 1 {
        let buff_snapshot = ctx.managers.buff_mgr.clone();
        ctx.managers.buff_mgr.tick_round_end();
        let out = broadcast::collect_buff_tick_broadcast(ctx, true);
        ctx.managers.buff_mgr = buff_snapshot;
        out
    } else {
        broadcast::collect_buff_tick_broadcast(ctx, true)
    };
    broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, true);
    if injected_channel_buffs {
        broadcast::adjust_attacker_round1_broadcast_uids(&mut broadcast);
    }
    broadcast
}

/// Build the terminal-round attacker broadcast. If the natural
/// broadcast already contains the `530000112` boss-state-cycle buff,
/// return it unchanged. Otherwise, when the battle's
/// `addition_rule` references the `530000151` boss cycle skill,
/// synthesize one `530000112` BuffUpdate per alive attacker so the
/// shape matches what the official client emits at battle end.
pub(crate) fn collect_terminal_round_end_broadcast(
    ctx: &mut FightContext<'_>,
) -> Vec<ActEffect> {
    let broadcast = collect_attacker_round_end_broadcast(ctx, false, true);
    if broadcast.iter().any(|effect| {
        effect
            .buff
            .as_ref()
            .and_then(|buff| buff.buff_id)
            .unwrap_or(0)
            == 530000112
    }) {
        return broadcast;
    }

    if !collect_battle_rule_skills(ctx.fight)
        .contains(&530000151)
    {
        return broadcast;
    }

    let mut synthesized = Vec::new();
    if let Some(attacker) = ctx.fight.attacker.as_ref() {
        for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
            if entity.position.unwrap_or(-1) <= 0 || entity.current_hp.unwrap_or(0) <= 0 {
                continue;
            }
            let Some(uid) = entity.uid else { continue };
            let buff_uid = next_buff_uid_for_target(uid);
            let mut effect = crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                uid, -1, 530000112, buff_uid, 0, 0,
            );
            if let Some(buff) = effect.buff.as_mut() {
                buff.duration = Some(1);
                buff.count = Some(0);
            }
            synthesized.push(effect);
        }
    }

    if synthesized.is_empty() {
        broadcast
    } else {
        synthesized
    }
}

/// Walk attacker-side passives and return the first one that emits
/// a non-empty effect set under the `NoActRound` condition (skill
/// behavior condition `46301`). Used by the terminal-round emitter
/// to surface the one ally passive that closes out the round.
pub(crate) fn build_terminal_attacker_round_end_passive_step(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
) -> Option<FightStep> {
    let passive_phase = ctx.combat_phase();
    let battle_rule_skills = collect_battle_rule_skills(ctx.fight);

    for uid in collected.attacker_uids() {
        for skill_id in collected.merged_for(uid) {
            if battle_rule_skills.contains(&skill_id) || !skill_has_no_act_round_condition(skill_id)
            {
                continue;
            }
            if let Ok(effects) = execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                && !effects.is_empty()
            {
                return Some(build_effect_step(effects));
            }
        }
    }

    None
}

/// Merge solitary post-round-end reactive wrappers back into the
/// player-card host they belong to. After the `effect_type=276`
/// round-end marker, any single-effect wrapper whose inner SKILL
/// has a positive `from_id` and matches a player-card host that
/// fired earlier in the round gets folded back into that host's
/// `act_effect`. The duplicate-guard skips wrappers whose
/// (act_id, from_id) already exist on the host.
///
/// The 2-merge floor is intentional: solitary post-round wrappers
/// occur in fights other than the battle2 burst this helper was
/// originally written for; under-2 cases stay top-level so they
/// don't get prematurely absorbed into a host the LIVE client
/// emits separately.
pub(crate) fn merge_post_turn_reactives_into_host(steps: &mut Vec<FightStep>) {
    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(276)
    }) else {
        return;
    };

    let mut player_card_hosts: HashMap<i64, usize> = HashMap::new();
    for (idx, step) in steps.iter().enumerate().take(round_end_idx) {
        if step.act_type != Some(fight_step::ActType::Skill as i32) {
            continue;
        }
        let from_id = step.from_id.unwrap_or(0);
        if from_id > 0 {
            player_card_hosts.insert(from_id, idx);
        }
    }
    if player_card_hosts.is_empty() {
        return;
    }

    let mut merges: Vec<(usize, usize, ActEffect)> = Vec::new();
    for (source_idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        if step.act_type != Some(fight_step::ActType::Effect as i32)
            || step.act_effect.len() != 1
            || step
                .act_effect
                .first()
                .and_then(|effect| effect.effect_type)
                != Some(162)
        {
            break;
        }

        let Some(wrapper) = step.act_effect.first().cloned() else {
            break;
        };

        let Some(reactive_step) = wrapper.fight_step.as_ref() else {
            continue;
        };
        if reactive_step.act_type != Some(fight_step::ActType::Skill as i32) {
            continue;
        }

        let player_uid = reactive_step.from_id.unwrap_or(0);
        if player_uid <= 0 {
            continue;
        }

        let Some(&target_idx) = player_card_hosts.get(&player_uid) else {
            continue;
        };
        merges.push((source_idx, target_idx, wrapper));
    }

    // Live battle2 leaks a burst of player-owned post-round wrappers here;
    // solitary wrappers still occur in other fights and stay top-level.
    if merges.len() < 2 {
        return;
    }

    for (_, target_idx, wrapper) in merges.iter().cloned() {
        if let Some(host_step) = steps.get_mut(target_idx) {
            let incoming_act_id = wrapper.fight_step.as_ref().and_then(|step| step.act_id);
            let incoming_from_id = wrapper.fight_step.as_ref().and_then(|step| step.from_id);
            let already_present = host_step.act_effect.iter().any(|existing| {
                existing.effect_type == Some(162)
                    && existing
                        .fight_step
                        .as_ref()
                        .map(|step| {
                            step.act_type == Some(fight_step::ActType::Skill as i32)
                                && step.act_id == incoming_act_id
                                && step.from_id == incoming_from_id
                        })
                        .unwrap_or(false)
            });
            if already_present {
                continue;
            }
            host_step.act_effect.push(wrapper);
        }
    }

    for source_idx in merges
        .into_iter()
        .map(|(source_idx, _, _)| source_idx)
        .rev()
    {
        steps.remove(source_idx);
    }
}

/// LIVE keeps Sotheby's `30090146` and Willow's standalone
/// `31040141` late-tail passive overflow under one post-`212`
/// compound wrapper instead of surfacing them as separate top-level
/// steps after the round-end `276` marker. Re-host the surviving raw
/// wrappers after the full round has been assembled so the fix stays
/// phase-local and does not disturb runtime passive execution order.
pub(crate) fn coalesce_late_tail_exclude_battle_rule_passives(steps: &mut Vec<FightStep>) -> bool {
    let Some(round_end_idx) = steps
        .iter()
        .position(|step| step_walker::is_standalone_effect_marker(step, 276))
    else {
        return false;
    };
    let Some(post_212_idx) = steps[round_end_idx + 1..]
        .iter()
        .position(|step| step_walker::is_standalone_effect_marker(step, CHANGE_ROUND_BUNDLE_MARKER))
        .map(|off| round_end_idx + 1 + off)
    else {
        return false;
    };

    let mut sotheby_wrappers: Vec<ActEffect> = Vec::new();
    let mut willow_sources: Vec<(usize, ActEffect, Vec<ActEffect>)> = Vec::new();
    let mut remove_idxs: Vec<usize> = Vec::new();

    let target_idx = steps[post_212_idx + 1..]
        .iter()
        .position(|step| standalone_overflow_skill_id(step) == Some(WILLOW_LATE_FRAGMENT_SKILL_ID))
        .map(|off| post_212_idx + 1 + off);

    for (idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        let Some(skill_id) = standalone_overflow_skill_id(step) else {
            continue;
        };
        if target_idx == Some(idx) {
            continue;
        }
        match skill_id {
            SOTHEBY_LATE_GRANT_SKILL_ID => {
                if let Some(wrapper) = step.act_effect.first().cloned() {
                    sotheby_wrappers.push(wrapper);
                    remove_idxs.push(idx);
                }
            }
            WILLOW_LATE_FRAGMENT_SKILL_ID => {
                let Some(wrapper) = step.act_effect.first().cloned() else {
                    continue;
                };
                let payload = step
                    .act_effect
                    .first()
                    .and_then(step_walker::wrapped_skill_from_effect)
                    .map(|skill| skill.act_effect.clone())
                    .unwrap_or_default();
                willow_sources.push((idx, wrapper, payload));
                remove_idxs.push(idx);
            }
            _ => {}
        }
    }

    if sotheby_wrappers.is_empty() && willow_sources.is_empty() {
        return false;
    }

    let target_idx = match target_idx {
        Some(idx) => idx,
        None => {
            let Some((_, base_wrapper, _)) = willow_sources.pop() else {
                return false;
            };
            let insert_idx = steps[post_212_idx + 1..]
                .iter()
                .position(|step| {
                    step_walker::is_standalone_effect_marker(step, 310)
                        || step_walker::step_contains_act_id(step, 30091111)
                        || step_walker::step_contains_act_id(step, 30091122)
                        || step_walker::step_contains_act_id(step, 30091123)
                })
                .map(|off| post_212_idx + 1 + off)
                .unwrap_or(steps.len());
            steps.insert(insert_idx, build_effect_step(vec![base_wrapper]));
            insert_idx
        }
    };

    let Some(target_step) = steps.get_mut(target_idx) else {
        return false;
    };
    let Some(willow_effect_idx) = target_step.act_effect.iter().position(|effect| {
        step_walker::wrapped_skill_from_effect(effect).map(|skill| skill.act_id)
            == Some(Some(WILLOW_LATE_FRAGMENT_SKILL_ID))
    }) else {
        return false;
    };

    if let Some(willow_skill) =
        step_walker::wrapped_skill_from_effect_mut(&mut target_step.act_effect[willow_effect_idx])
    {
        for (_, _, payload) in willow_sources {
            willow_skill.act_effect.extend(payload);
        }
    }

    if !sotheby_wrappers.is_empty()
        && !target_step.act_effect.iter().any(|effect| {
            step_walker::wrapped_skill_from_effect(effect).map(|skill| skill.act_id)
                == Some(Some(SOTHEBY_LATE_GRANT_SKILL_ID))
        })
    {
        for wrapper in sotheby_wrappers.into_iter().rev() {
            target_step.act_effect.insert(willow_effect_idx, wrapper);
        }
    }

    remove_idxs.sort_unstable();
    remove_idxs.dedup();
    for source_idx in remove_idxs.into_iter().rev() {
        steps.remove(source_idx);
    }

    true
}

/// LIVE battle1 r1 re-broadcasts the current boss-state snapshot
/// immediately after the round-end `4150001` wrapper and before the
/// `SmallRoundEnd(211)` marker. The packet is not a fresh generic
/// passive execution at this checkpoint; it is a replay of the active
/// `530000111` state on defenders that still carry that buff after the
/// round-end transition. That naturally excludes battle1's `-3`.
pub(crate) fn repair_boss_state_cycle_second_wave(
    ctx: &mut FightContext<'_>,
    steps: &mut Vec<FightStep>,
) -> bool {
    if !collect_battle_rule_skills(ctx.fight)
        .contains(&BOSS_STATE_CYCLE_SKILL_ID)
    {
        return false;
    }

    let Some(small_round_end_idx) = steps.iter().position(|step| {
        step_walker::is_standalone_effect_marker(step, SMALL_ROUND_END_EFFECT_TYPE)
    }) else {
        return false;
    };

    let Some(state_marker_idx) = steps[..small_round_end_idx]
        .iter()
        .rposition(|step| step_walker::step_contains_act_id(step, ROUND_END_STATE_MARKER_SKILL_ID))
    else {
        return false;
    };

    if steps[state_marker_idx + 1..small_round_end_idx]
        .iter()
        .any(step_contains_boss_state_cycle_second_wave)
    {
        return false;
    }

    let mut wrapped = Vec::new();
    let defender_uids: Vec<i64> = ctx
        .fight
        .defender
        .as_ref()
        .map(|side| {
            side.entitys
                .iter()
                .chain(side.sub_entitys.iter())
                .filter(|entity| {
                    entity.position.unwrap_or(0) > 0
                        && entity.passive_skill.contains(&BOSS_STATE_CYCLE_SKILL_ID)
                })
                .filter_map(|entity| entity.uid)
                .collect()
        })
        .unwrap_or_default();

    for uid in defender_uids {
        let Some(snapshot) = boss_state_snapshot_for_uid(ctx, &steps[..small_round_end_idx], uid)
        else {
            continue;
        };

        let mut update = crate::state::battle::fight_step::ActEffectBuilder::buff_update(
            uid,
            snapshot.from_uid,
            BOSS_STATE_ACTIVE_BUFF_ID,
            snapshot.buff_uid,
            snapshot.count,
            snapshot.layer,
        );
        if let Some(buff) = update.buff.as_mut() {
            buff.duration = Some(snapshot.duration);
        }
        let skill = FightStepBuilder::skill(uid, uid, BOSS_STATE_CYCLE_SKILL_ID)
            .with(update)
            .with(ActEffectBuilder::attr_with_num(uid, 0))
            .build();
        wrapped.push(wrap_step(build_effect_step(vec![wrap_step(skill)])));
    }

    if wrapped.is_empty() {
        return false;
    }

    steps.insert(small_round_end_idx, build_effect_step(wrapped));
    true
}

/// LIVE battle2 r1 surfaces Rubuska's round-end `31250144` heal
/// pulse as four standalone top-level Heal markers immediately after
/// the `31250151` Shadow Friend bundle and before the consolidated HP
/// snapshot. Our flat-sweep path applies the passive to state but the
/// visible markers do not survive the later round-end cleanup chain,
/// so restore just the four bare `et=4` packets here.
pub(crate) fn repair_rubuska_round_end_heal_markers(
    ctx: &FightContext<'_>,
    steps: &mut Vec<FightStep>,
) -> bool {
    let rubuska_uid = ctx.mechanics.shadow_cloak.rubuska_uid;
    if !rubuska::has_round_end_heal_pulse(&ctx.managers.buff_mgr, rubuska_uid)
        && !steps.iter().any(step_contains_rubuska_heal_pulse_context)
    {
        return false;
    }

    let Some((bundle_idx, targets)) = find_round_end_shadow_friend_targets(steps, rubuska_uid)
    else {
        return false;
    };
    if targets.is_empty() || heal_marker_block_matches(steps, bundle_idx + 1, &targets) {
        return false;
    }

    for target_uid in targets.into_iter().rev() {
        steps.insert(
            bundle_idx + 1,
            build_effect_step(vec![ActEffectBuilder::heal(target_uid, 0, None)]),
        );
    }
    true
}

fn step_contains_boss_state_cycle_second_wave(step: &FightStep) -> bool {
    step.act_effect.iter().any(|effect| {
        let Some(skill) = step_walker::wrapped_skill_from_effect(effect) else {
            return false;
        };
        let Some(uid) = skill.from_id else {
            return false;
        };
        uid < 0 && skill_matches_boss_state_cycle_second_wave(skill, uid)
    })
}

fn standalone_overflow_skill_id(step: &FightStep) -> Option<i32> {
    (step.act_type == Some(fight_step::ActType::Effect as i32) && step.act_effect.len() == 1)
        .then(|| step.act_effect.first())
        .flatten()
        .and_then(step_walker::wrapped_skill_from_effect)
        .and_then(|skill| skill.act_id)
}

fn boss_state_snapshot_for_uid(
    ctx: &FightContext<'_>,
    steps: &[FightStep],
    uid: i64,
) -> Option<BossStateSnapshot> {
    if let Some(instance) = ctx
        .managers
        .buff_mgr
        .find_instance_by_buff_id(uid, BOSS_STATE_ACTIVE_BUFF_ID)
    {
        return Some(BossStateSnapshot {
            buff_uid: instance.uid,
            from_uid: instance.from_uid,
            count: instance.stacks,
            layer: instance.layer,
            duration: instance.duration,
        });
    }

    if ctx
        .managers
        .buff_mgr
        .find_instance_by_buff_id(uid, BOSS_STATE_INACTIVE_BUFF_ID)
        .is_some()
    {
        return None;
    }

    steps.iter().rev().find_map(|step| {
        step.act_effect.iter().find_map(|effect| {
            let skill = step_walker::wrapped_skill_from_effect(effect)?;
            if skill.act_id != Some(BOSS_STATE_CYCLE_SKILL_ID) || skill.from_id != Some(uid) {
                return None;
            }
            let first = skill.act_effect.first()?;
            let buff = first.buff.as_ref()?;
            (first.effect_type == Some(7) && buff.buff_id == Some(BOSS_STATE_ACTIVE_BUFF_ID)).then(
                || BossStateSnapshot {
                    buff_uid: buff.uid.unwrap_or(0),
                    from_uid: buff.from_uid.unwrap_or(uid),
                    count: buff.count.unwrap_or(0),
                    layer: buff.layer.unwrap_or(0),
                    duration: buff.duration.unwrap_or(0),
                },
            )
        })
    })
}

fn skill_matches_boss_state_cycle_second_wave(skill: &FightStep, uid: i64) -> bool {
    skill.act_id == Some(BOSS_STATE_CYCLE_SKILL_ID)
        && skill.from_id == Some(uid)
        && skill.to_id == Some(uid)
        && skill.act_effect.len() == 2
        && skill
            .act_effect
            .first()
            .map(|effect| effect.effect_type == Some(7) && effect.target_id == Some(uid))
            .unwrap_or(false)
        && skill
            .act_effect
            .get(1)
            .map(|effect| effect.effect_type == Some(26) && effect.target_id == Some(uid))
            .unwrap_or(false)
}

fn find_round_end_shadow_friend_targets(
    steps: &[FightStep],
    rubuska_uid: i64,
) -> Option<(usize, Vec<i64>)> {
    steps.iter().enumerate().rev().find_map(|(idx, step)| {
        shadow_friend_bundle_targets(step, rubuska_uid).map(|targets| (idx, targets))
    })
}

fn shadow_friend_bundle_targets(step: &FightStep, rubuska_uid: i64) -> Option<Vec<i64>> {
    if step.act_type != Some(fight_step::ActType::Effect as i32)
        || step.act_id != Some(0)
        || step.from_id != Some(0)
        || step.to_id != Some(0)
        || step.act_effect.is_empty()
    {
        return None;
    }

    let mut targets = Vec::with_capacity(step.act_effect.len());
    for effect in &step.act_effect {
        if effect.effect_type != Some(EffectType::Fightstep as i32) {
            return None;
        }
        let Some(inner) = effect.fight_step.as_ref() else {
            return None;
        };
        if inner.act_type != Some(fight_step::ActType::Effect as i32)
            || inner.act_id != Some(rubuska::SHADOW_CLOAK_ACCUMULATOR_BUFF_ID)
            || inner.from_id != Some(rubuska_uid)
        {
            return None;
        }
        let target_uid = inner.to_id.unwrap_or(0);
        if target_uid <= 0 || targets.contains(&target_uid) {
            return None;
        }
        targets.push(target_uid);
    }

    Some(targets)
}

fn heal_marker_block_matches(steps: &[FightStep], start_idx: usize, targets: &[i64]) -> bool {
    targets.iter().enumerate().all(|(offset, target_uid)| {
        steps
            .get(start_idx + offset)
            .is_some_and(|step| is_standalone_heal_marker(step, *target_uid))
    })
}

fn is_standalone_heal_marker(step: &FightStep, target_uid: i64) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_id == Some(0)
        && step.from_id == Some(0)
        && step.to_id == Some(0)
        && step.act_effect.len() == 1
        && step
            .act_effect
            .first()
            .map(|effect| {
                effect.fight_step.is_none()
                    && effect.effect_type == Some(EffectType::Heal as i32)
                    && effect.target_id == Some(target_uid)
            })
            .unwrap_or(false)
}

fn step_contains_rubuska_heal_pulse_context(step: &FightStep) -> bool {
    if step.act_id == Some(31250144) {
        return true;
    }

    step.act_effect
        .iter()
        .any(effect_contains_rubuska_heal_pulse_context)
}

fn effect_contains_rubuska_heal_pulse_context(effect: &ActEffect) -> bool {
    if effect.buff.as_ref().and_then(|buff| buff.buff_id)
        == Some(rubuska::SHADOW_CLOAK_HEAL_PULSE_TYPE_ID)
    {
        return true;
    }

    effect
        .fight_step
        .as_ref()
        .map(step_contains_rubuska_heal_pulse_context)
        .unwrap_or(false)
}
