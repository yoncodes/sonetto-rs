//! Non-terminal round phase: orchestrates the post-player-turn
//! sequence — round-end transition marker, attacker/defender passive
//! sweeps, enemy actions, channel followups, defender round-end
//! tick broadcast, DOT/HoT settlement, wave advancement, and
//! post-round-start sweeps.
//!
//! This is the longest phase function in the round manager. It only
//! runs when `state.is_finish` is false at entry; if the round is
//! already terminal, control delegates to
//! `round_end_emission::emit_terminal_round_steps` immediately.

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, CardInfo, FightStep};

use crate::state::battle::{
    buff_actions::{self, BuffStage, round_end as round_end_handler},
    context::FightContext,
    deck::DeckManager,
    fight_step::{ActEffectBuilder, FightStepBuilder, effect_container_step, wrap_step},
    heroes::rubuska,
    manager::{
        buff_mgr::{LifecycleEventKind, reset_buff_uid_to},
        entity_mgr::sync_from_fight,
        round_mgr::{
            BattleEndState, apply_passive_phase, apply_step_and_maybe_sync, check_battle_end,
            check_battle_state, collect_round_tied_defender_passive_steps,
            deleted_buff_ids_from_delta, expand_trigger_chain, first_alive_defender_uid,
            get_max_wave, seed_entry_max_hp_from_fight,
        },
        traits::Manager,
    },
    mechanics::{advanced_cure, bloodtithe, channel as channel_mechanics},
    passives::{self, collector::CollectedPassives},
    phase,
    round::{
        PassivePhaseConfig, PhaseDepth, PhaseScope, PhaseSkillSet, PhaseStepShape, RoundState,
        step_shape::build_effect_step, steps::transitions::build_pre_enemy_transition_steps,
    },
    round_end_emission,
    skill::SkillExecutor,
    step_walker,
    steps::{broadcast, ex_gain, step_normalize},
    types::effects::EffectType,
    utils::buff_get_act_common_params,
};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    executor: &mut SkillExecutor,
    state: &mut RoundState,
    selected_for_round_end: Vec<CardInfo>,
    collected: &CollectedPassives,
    defender_uid_checkpoint: i64,
    deck_mgr: &mut DeckManager,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    if state.is_finish {
        round_end_emission::emit_terminal_round_steps(
            ctx,
            selected_for_round_end,
            collected,
            steps,
        )?;
        return Ok(());
    }

    // Player turn finished; emit round-end transition marker (live parity).
    steps.push(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::allocate_card_energy(
                selected_for_round_end,
            ))
            .build(),
    );
    // Late-round attacker passive sweep — runs every attacker c100/c101/c102/c104
    // round-start passive AFTER player actions, matching LIVE's bundling at
    // depth-2 wrappers like battle3 r2 root[24] / r3 root[21] (which carry
    // both `30090146` Sotheby grant and `31040141` Willow Poison-apply
    // together). The earlier `is_single_slot_pure_c100_passive` filter
    // excluded pure-c100 here because they used to fire pre-player at
    // `round_mgr.rs:551`; that sweep was removed in Phase 6 Session 4.47
    // so pure-c100 (e.g. Sotheby's `30090146` Duality Potion grant) now
    // fires here too, restoring LIVE's holder-layer-vs-attack-time
    // ordering.
    apply_passive_phase(
        ctx,
        collected,
        PassivePhaseConfig {
            scope: PhaseScope::Attackers,
            depth: PhaseDepth::FirstMatch,
            skill_set: PhaseSkillSet::ExcludeBattleRule,
            step_shape: PhaseStepShape::Raw,
        },
        true,
        steps,
    )?;
    steps.extend(build_pre_enemy_transition_steps(
        deck_mgr.player_deck.len() as i32
    ));
    if let Some((dead_uid, sub_entity, position)) = ctx.managers.entity_mgr.sub_hero(ctx.fight) {
        let sub_uid = sub_entity.uid.unwrap_or(0);
        ctx.managers.passive_mgr.seed_entity(&sub_entity);
        if sub_uid != 0 {
            let fight = &*ctx.fight;
            ctx.managers.rule_mgr.seed_entity_uid(sub_uid, fight);
        }
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffectBuilder::change_hero(
                    dead_uid, sub_entity, position,
                ))
                .build(),
        );
        if sub_uid != 0 {
            let events = ctx.on_enter_fight(sub_uid);
        }
    }
    let defender_bootstrap_start = steps.len();
    apply_passive_phase(
        ctx,
        collected,
        PassivePhaseConfig {
            scope: PhaseScope::Defenders,
            depth: PhaseDepth::AllMatches,
            skill_set: PhaseSkillSet::DefenderBootstrap,
            step_shape: PhaseStepShape::Raw,
        },
        true,
        steps,
    )?;
    let boss_wrappers = collect_round_tied_defender_passive_steps(ctx, collected);
    if !boss_wrappers.is_empty()
        && let Some(boss_subtree) =
            step_walker::find_bootstrap_nested_effects_mut(&mut steps[defender_bootstrap_start..])
    {
        boss_subtree.extend(boss_wrappers);
        let reactive_target_skills =
            channel_mechanics::gather_boss_invoked_reactive_target_skills(ctx.fight);
        // Walker (passives::inject) handles depth-first recursion +
        // identifying enemy SKILL emission slots. Per-step injector
        // (channel_mechanics::inject_monitor_continue_into_enemy_skill_step)
        // owns Sentinel's holder lookup, channel-buff wrapping, and
        // layer-counter consumption.
        passives::inject::inject_ally_reactives_into_enemy_subtree(
            ctx,
            collected,
            boss_subtree,
            &|ctx, collected, step| {
                passives::ally_be_attacked::inject_into_enemy_skill_step(
                    ctx,
                    collected,
                    step,
                    &|ctx, reactive_step| {
                        if let Err(err) = apply_step_and_maybe_sync(ctx, reactive_step, true) {
                            tracing::warn!(
                                "ally be_attacked inline reactive apply failed act_id={:?}: {}",
                                reactive_step.act_id,
                                err
                            );
                        }
                    },
                );
                channel_mechanics::inject_monitor_continue_into_enemy_skill_step(
                    ctx,
                    collected,
                    step,
                    &reactive_target_skills,
                    &|ctx, collected, root, deleted| {
                        expand_trigger_chain(ctx, collected, root, deleted)
                    },
                    &|before, after| deleted_buff_ids_from_delta(before, after),
                );
            },
        );
    }

    reset_buff_uid_to(defender_uid_checkpoint);

    // Enemy actions
    phase::enemy_actions::run(rng, ctx, executor, state, collected, steps).await?;

    let injected_channel_buffs =
        channel_mechanics::inject_channel_followup_buffs_if_missing(ctx, collected, steps);

    // Live parity: run a passive combat sweep for defender side after AI actions.
    // This emits nested trigger/follow-up 162 steps before round-end transitions.
    let defender_sweep_start = steps.len();
    apply_passive_phase(
        ctx,
        collected,
        PassivePhaseConfig {
            scope: PhaseScope::Defenders,
            depth: PhaseDepth::AllMatches,
            skill_set: PhaseSkillSet::ExcludeBattleRule,
            step_shape: PhaseStepShape::Raw,
        },
        true,
        steps,
    )?;

    // Live parity: append defender round-end buff tick broadcast. Live
    // emits one FightStep containing (a) a 162 wrapper for the first
    // passive-firing defender and (b) one BuffUpdate per duration==1 buff
    // across alive defenders. Our sweep currently emits multiple 162
    // wrappers; merge them and append the BuffUpdate snapshot so the
    // shape matches live once upstream buffs are emitted correctly.
    {
        // Live broadcasts tick-expiring buffs with remaining duration=1.
        // Our manager decrements durations at true round-end; preview one tick
        // here for packet shaping, then restore runtime state.
        // TODO(event-queue): the snapshot/restore preview pattern is a
        // EventQueue Phase 5 migration target — replace with a typed
        // PreviewRoundEndTick event that records the desired snapshot
        // without committing buff_mgr state.
        let mut broadcast = if ctx.fight.cur_round.unwrap_or(1) == 1 {
            let buff_snapshot = ctx.managers.buff_mgr.clone();
            ctx.managers.buff_mgr.tick_round_end();
            let out = broadcast::collect_buff_tick_broadcast(ctx, false);
            ctx.managers.buff_mgr = buff_snapshot;
            out
        } else {
            broadcast::collect_buff_tick_broadcast(ctx, false)
        };
        broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, false);
        broadcast::adjust_defender_round1_broadcast_uids(ctx, &mut broadcast);
        if !broadcast.is_empty()
            && let Some(target_idx) = steps[defender_sweep_start..]
                .iter()
                .position(|s| {
                    s.act_effect
                        .iter()
                        .any(|e| e.effect_type == Some(EffectType::FightStep as i32))
                })
                .map(|off| defender_sweep_start + off)
        {
            // Keep only the first 162 wrapper of this step and append the
            // BuffUpdate broadcast after it.
            let preferred_wrapper = steps[..=target_idx]
                .iter()
                .rev()
                .flat_map(|s| s.act_effect.iter())
                .find(|e| step_normalize::is_preferred_defender_round_end_wrapper(ctx.fight, e))
                .cloned();
            let target = &mut steps[target_idx];

            let broadcast_anchor_uid = broadcast
                .iter()
                .filter_map(|e| e.buff.as_ref().and_then(|b| b.uid))
                .min();
            let first_wrapper = preferred_wrapper
                .or_else(|| {
                    target
                        .act_effect
                        .iter()
                        .find(|e| {
                            step_normalize::is_preferred_defender_round_end_wrapper(ctx.fight, e)
                        })
                        .cloned()
                })
                .or_else(|| {
                    target
                        .act_effect
                        .iter()
                        .find(|e| e.effect_type == Some(EffectType::FightStep as i32))
                        .cloned()
                })
                .map(|wrapper| {
                    step_normalize::normalize_defender_round_end_wrapper(
                        ctx,
                        wrapper,
                        broadcast_anchor_uid,
                    )
                });
            if let Some(first_wrapper) = first_wrapper {
                let mut new_effects = vec![first_wrapper];
                new_effects.extend(broadcast);
                target.act_effect = new_effects;
            }
        }
    }

    if let Some(step) = round_end_handler::build_round_end_lost_hp_count_add_buff_step(ctx) {
        apply_step_and_maybe_sync(ctx, &step, true)?;
        steps.push(step);
    }

    let dot_effects = {
        let mut dot_executor = SkillExecutor::new();
        let mut dot_effect_ctx =
            buff_actions::EffectContext::new(ctx.fight, ctx.managers, ctx.mechanics, 0, 0);
        let mut dispatch_ctx =
            buff_actions::DispatchCtx::new(&mut dot_effect_ctx, &mut dot_executor);
        buff_actions::dispatch_stage(BuffStage::RoundEndDot, &mut dispatch_ctx)
    };
    if !dot_effects.is_empty() {
        let mut step = build_effect_step(dot_effects);
        buff_actions::dedupe_dead_effects_against_prior_steps(&mut step, steps);
        apply_step_and_maybe_sync(ctx, &step, true)?;
        steps.push(step);
    }

    let cur_wave = ctx.fight.cur_wave.unwrap_or(1);
    let max_wave = get_max_wave(ctx.fight);
    let battle_state = check_battle_state(ctx.fight, cur_wave, max_wave);
    let wave_cleared = matches!(battle_state, BattleEndState::WaveCleared);
    if matches!(
        battle_state,
        BattleEndState::Victory | BattleEndState::Defeat
    ) {
        return Ok(());
    }

    // End of enemy turn transition.
    steps.push(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::small_round_end(None, 1))
            .build(),
    );
    if let Some((dead_uid, sub_entity, position)) = ctx.managers.entity_mgr.sub_hero(ctx.fight) {
        let sub_uid = sub_entity.uid.unwrap_or(0);
        ctx.managers.passive_mgr.seed_entity(&sub_entity);
        if sub_uid != 0 {
            let fight = &*ctx.fight;
            ctx.managers.rule_mgr.seed_entity_uid(sub_uid, fight);
        }
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffectBuilder::change_hero(
                    dead_uid, sub_entity, position,
                ))
                .build(),
        );
        if sub_uid != 0 {
            steps.extend(crate::state::battle::event::apply::events_to_steps(ctx.on_enter_fight(sub_uid)));
        }
    }
    if let Some(caster_uid) = first_alive_defender_uid(ctx.fight)
        && state.enemy_skill_actors.contains(&caster_uid)
        && let Some(ex_step) = ex_gain::standard_action_ex_gain_for_uid(ctx, caster_uid)
    {
        steps.push(ex_step);
    }

    // Round transition markers.
    steps.push(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::clear_universal_card(None, None, Some(1)))
            .build(),
    );
    deck_mgr.clear_universal_card();
    // Skip the magic-circle duration tick only when the battle
    // itself is finishing (state.is_finish or check_battle_end true).
    // LIVE keeps ticking the circle through wave-clear rounds — the
    // tick fires BEFORE wave-spawn even when the current wave just
    // got wiped out (see battle3 r5 step[21] tick → step[22] et=337
    // wave-spawn). Self-only circles (Semmelweis 100051) are still
    // skipped inside `build_round_end_magic_circle_step` via the
    // `has_enemy_side` config check, so battle2 r2 stays clean.
    if !state.is_finish && !check_battle_end(ctx.fight) {
        if let Some(step) = build_round_end_magic_circle_step(ctx) {
            apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }
    }
    if wave_cleared {
        let old_defender_uids: Vec<i64> = ctx
            .fight
            .defender
            .as_ref()
            .into_iter()
            .flat_map(|defender| defender.entitys.iter().chain(defender.sub_entitys.iter()))
            .filter_map(|entity| entity.uid)
            .collect();
        let mut wave_executor = SkillExecutor::new();
        let mut wave_mgr = std::mem::take(&mut ctx.managers.wave_mgr);
        let wave_steps = wave_mgr.advance_wave(ctx, &mut wave_executor)?;
        ctx.managers.wave_mgr = wave_mgr;
        sync_from_fight(ctx.fight, &mut ctx.managers.entity_mgr);
        for uid in old_defender_uids {
            ctx.managers.buff_mgr.clear(uid);
        }
        seed_entry_max_hp_from_fight(ctx.fight);
        ctx.sync();
        steps.extend(wave_steps);
    }
    steps.push(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::change_round(None, None))
            .build(),
    );
    // New-round boundary: reset per-slot round-limit usage trackers before
    // post-round-start passive sweeps execute.
    ctx.managers.buff_mgr.reset_skill_slot_round_usage();
    let round_start_tail_start = steps.len();
    run_post_change_round_tail(
        ctx,
        collected,
        steps,
        deck_mgr.player_deck.len() as i32,
        injected_channel_buffs,
    )?;

    // LIVE still applies the downstream Shadow Cloak / bloodpool state updates
    // on terminal transitions, but it does not surface the visible
    // `31250151` Shadow Friend HP-loss wrapper when the same round-start tail
    // ends the battle. Suppress only that packet shape after the full tail
    // resolves so runtime state remains unchanged.
    if check_battle_end(ctx.fight) {
        suppress_terminal_shadow_friend_packets(steps, round_start_tail_start);
    }

    Ok(())
}

fn run_post_change_round_tail(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
    deck_num: i32,
    injected_channel_buffs: bool,
) -> Result<()> {
    let tail_start = steps.len();

    // Battle2 bloodtithe parity: live re-runs the same blood-pool pipeline
    // here that battle start uses before the next-round attacker sweep.
    for step in bloodtithe::build_round_transition_bloodtithe_steps(ctx, collected) {
        apply_step_and_maybe_sync(ctx, &step, true)?;
        steps.push(step);
    }

    // Post-round-start battle-rule passives on attacker side (e.g. global rule skills).
    apply_passive_phase(
        ctx,
        collected,
        PassivePhaseConfig {
            scope: PhaseScope::Attackers,
            depth: PhaseDepth::AllMatches,
            skill_set: PhaseSkillSet::BattleRuleOnly,
            step_shape: PhaseStepShape::Raw,
        },
        false,
        steps,
    )?;

    // Post-round-start attacker sweep.
    let attacker_sweep_start = steps.len();
    apply_passive_phase(
        ctx,
        collected,
        PassivePhaseConfig {
            scope: PhaseScope::Attackers,
            depth: PhaseDepth::AllMatches,
            skill_set: PhaseSkillSet::CombatReactive,
            step_shape: PhaseStepShape::FlatIfAllUpdate,
        },
        true,
        steps,
    )?;

    // Live parity: overwrite the flat BuffUpdate step emitted by the sweep
    // (which only carries one passive's output) with a full snapshot of
    // every alive attacker's duration==1 buffs. This matches the live
    // "round-end tick" broadcast shape (one FightStep with one BuffUpdate
    // per expiring buff across the side).
    {
        let mut broadcast = round_end_emission::collect_attacker_round_end_broadcast(
            ctx,
            injected_channel_buffs,
            false,
        );
        if injected_channel_buffs && broadcast.len() > 6 {
            broadcast.truncate(6);
        }
        if !broadcast.is_empty()
            && let Some(flat_idx) = steps[attacker_sweep_start..]
                .iter()
                .rposition(|s| {
                    !s.act_effect.is_empty()
                        && s.act_effect
                            .iter()
                            .all(|e| e.effect_type == Some(EffectType::BuffUpdate as i32))
                })
                .map(|off| attacker_sweep_start + off)
        {
            steps[flat_idx].act_effect = broadcast;
        }
    }

    // Round-end AdvancedCure HoT settlement — emits one 162-wrapped
    // skill fightStep per (target, buff_id, caster) triple where
    // the target carries an AdvancedCure buff. See
    // `mechanics/advanced_cure.rs` for the emission shape (marker
    // (0) + Heal (4)). The BuffUpdate(7) tail is intentionally
    // omitted; the existing round-end-tick broadcast collector
    // covers buff snapshot duties.
    if let Some(step) = advanced_cure::build_round_end_advanced_cure_step(ctx) {
        apply_step_and_maybe_sync(ctx, &step, true)?;
        steps.push(step);
    }

    // LIVE only emits the takeStage=103 lifecycle batch as a flat
    // top-level step when the post-212 tail already has a hero-owned
    // anchor wrapper. Battle2 instead folds those lifecycle entries
    // into the compound CardDeck delivery step, so preserving the
    // pre-4.31 baseline means suppressing the flat batch unless the
    // assembled tail already carries either Pickles' round-end wrapper
    // or an AdvancedCure wrapper.
    if tail_has_round_end_lifecycle_anchor(&steps[tail_start..])
        && let Some(step) = build_round_end_takestage_103_lifecycle_step(ctx)
    {
        steps.push(step);
    }

    // Next-round deck snapshot marker.
    steps.push(
        FightStepBuilder::effect()
            .with(ActEffectBuilder::card_deck_num(deck_num))
            .build(),
    );

    Ok(())
}

fn build_round_end_takestage_103_lifecycle_step(ctx: &FightContext<'_>) -> Option<FightStep> {
    let mut events_by_target: std::collections::HashMap<i64, Vec<_>> = ctx
        .managers
        .buff_mgr
        .preview_round_end_lifecycle_takestage_103()
        .into_iter()
        .filter(|event| event.target_uid > 0)
        .fold(std::collections::HashMap::new(), |mut acc, event| {
            acc.entry(event.target_uid).or_default().push(event);
            acc
        });
    let Some(attacker) = ctx.fight.attacker.as_ref() else {
        return None;
    };
    let mut effects = Vec::new();
    for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
        if entity.position.unwrap_or(-1) <= 0 || entity.current_hp.unwrap_or(0) <= 0 {
            continue;
        }
        let Some(target_uid) = entity.uid else {
            continue;
        };
        let Some(events) = events_by_target.remove(&target_uid) else {
            continue;
        };
        for event in events {
            let duration = match event.kind {
                LifecycleEventKind::Expiring => 0,
                LifecycleEventKind::Ticking => event.instance.duration - 1,
            };
            let act_common_params = buff_get_act_common_params(event.instance.buff_id);
            effects.push(match event.kind {
                LifecycleEventKind::Expiring => ActEffectBuilder::buff_del_with_snapshot(
                    event.target_uid,
                    event.instance.uid,
                    event.instance.buff_id,
                    event.instance.from_uid,
                    duration,
                    event.instance.stacks,
                    act_common_params,
                    event.instance.layer,
                    0,
                ),
                LifecycleEventKind::Ticking => ActEffectBuilder::buff_update_with_snapshot(
                    event.target_uid,
                    event.instance.from_uid,
                    event.instance.buff_id,
                    event.instance.uid,
                    duration,
                    event.instance.stacks,
                    act_common_params,
                    event.instance.layer,
                    0,
                ),
            });
        }
    }
    if effects.is_empty() {
        None
    } else {
        Some(build_effect_step(effects))
    }
}

const PICKLES_ROUND_END_SKILL_IDS: [i32; 2] = [30630151, 30630171];
const ADVANCED_CURE_BUFF_ACT_ID: i32 = 849;

fn tail_has_round_end_lifecycle_anchor(tail_steps: &[FightStep]) -> bool {
    tail_steps.iter().any(|step| {
        step_contains_pickles_round_end_anchor(step) || step_contains_advanced_cure_anchor(step)
    })
}

fn step_contains_pickles_round_end_anchor(step: &FightStep) -> bool {
    PICKLES_ROUND_END_SKILL_IDS
        .iter()
        .any(|act_id| step_walker::step_contains_act_id(step, *act_id))
}

fn step_contains_advanced_cure_anchor(step: &FightStep) -> bool {
    step.act_effect
        .iter()
        .any(effect_contains_advanced_cure_anchor)
}

fn effect_contains_advanced_cure_anchor(effect: &ActEffect) -> bool {
    if let Some(skill) = step_walker::wrapped_skill_from_effect(effect)
        && (skill
            .act_effect
            .iter()
            .any(|inner| inner.buff_act_id == Some(ADVANCED_CURE_BUFF_ACT_ID))
            || wrapped_skill_matches_advanced_cure_step_shape(skill))
    {
        return true;
    }

    effect
        .fight_step
        .as_ref()
        .map(step_contains_advanced_cure_anchor)
        .unwrap_or(false)
}

fn wrapped_skill_matches_advanced_cure_step_shape(skill: &FightStep) -> bool {
    if skill.act_effect.len() != 2 {
        return false;
    }
    let marker = &skill.act_effect[0];
    let heal = &skill.act_effect[1];
    marker.effect_type == Some(EffectType::None as i32)
        && heal.effect_type == Some(EffectType::Heal as i32)
        && marker.effect_num == skill.act_id
        && marker.target_id == skill.to_id
        && heal.target_id == skill.to_id
}

fn suppress_terminal_shadow_friend_packets(steps: &mut Vec<FightStep>, start_idx: usize) {
    let mut idx = start_idx;
    while idx < steps.len() {
        suppress_terminal_shadow_friend_effects(&mut steps[idx].act_effect);
        if steps[idx].act_effect.is_empty() {
            steps.remove(idx);
        } else {
            idx += 1;
        }
    }
}

fn suppress_terminal_shadow_friend_effects(effects: &mut Vec<ActEffect>) {
    let mut idx = 0;
    while idx < effects.len() {
        let remove = if let Some(step) = effects[idx].fight_step.as_mut() {
            if step.act_id == Some(rubuska::SHADOW_CLOAK_ACCUMULATOR_BUFF_ID) {
                true
            } else {
                suppress_terminal_shadow_friend_effects(&mut step.act_effect);
                step.act_effect.is_empty()
            }
        } else {
            false
        };

        if remove {
            effects.remove(idx);
        } else {
            idx += 1;
        }
    }
}

/// Emit the per-round magic-circle tick: counters with remaining
/// rounds get a `MagicCircleUpdate(140)`; expired ones (round
/// reaches 0) get a `MagicCircleDelete(139)` plus the cluster of
/// `BuffDel` for the circle's `enemy_buff` and the optional
/// `endSkills` marker. Self-only circles (no `enemy_buff` and no
/// `enemy_skills` in config) skip — they don't tick in the official
/// client either.
fn build_round_end_magic_circle_step(ctx: &mut FightContext<'_>) -> Option<FightStep> {
    let circle = ctx.fight.magic_circle.as_ref()?.clone();
    let current_round = circle.round.unwrap_or(0);
    if current_round <= 0 {
        return None;
    }

    let create_uid = circle.create_uid.unwrap_or(0);
    let circle_id = circle.magic_circle_id.unwrap_or(0);

    // Only circles that carry an enemy-side mechanic (enemy_buff or
    // enemy_skills) tick down per round in LIVE. Self-only circles
    // like Semmelweis's 100051 (`selfSkills`/`selfBuff` only) stay
    // un-ticked — they don't emit `MagicCircleUpdate(140)` per round
    // and are removed by other mechanisms (battle end, replacement
    // by another array). This keeps battle2 r2 byte-identical.
    let has_enemy_side = config::configs::get()
        .magic_circle
        .get(circle_id)
        .map(|cfg| !cfg.enemy_buff.trim().is_empty() || !cfg.enemy_skills.trim().is_empty())
        .unwrap_or(false);
    if !has_enemy_side {
        return None;
    }
    if current_round > 1 {
        let mut updated = circle;
        updated.round = Some(current_round - 1);
        let inner = effect_container_step(
            0,
            0,
            0,
            vec![ActEffectBuilder::magic_circle_update(
                create_uid, circle_id, "-1", updated,
            )],
        );
        return Some(build_effect_step(vec![wrap_step(inner)]));
    }

    let circle_cfg = config::configs::get().magic_circle.get(circle_id).cloned();
    let enemy_buff_id = circle_cfg
        .as_ref()
        .and_then(|cfg| cfg.enemy_buff.trim().parse::<i32>().ok())
        .filter(|id| *id > 0);
    let end_skills_id = circle_cfg
        .as_ref()
        .and_then(|cfg| cfg.end_skills.trim().parse::<i32>().ok())
        .filter(|id| *id > 0);

    // Snapshot alive enemies BEFORE building the cleanup so the
    // endSkills marker can target one of them — at this point the
    // BuffDel + Delete hasn't been applied yet, so the same enemies
    // that carry the enemy_buff are still on the field.
    let alive_enemy_uids =
        crate::state::battle::skill::targets::alive_enemies(ctx.fight, create_uid);

    let mut inner_effects = Vec::new();
    if let Some(buff_id) = enemy_buff_id {
        for enemy_uid in &alive_enemy_uids {
            if let Some(instance) = ctx
                .managers
                .buff_mgr
                .find_instance_by_buff_id(*enemy_uid, buff_id)
            {
                inner_effects.push(
                    crate::state::battle::fight_step::ActEffectBuilder::buff_del(
                        *enemy_uid,
                        instance.uid,
                        buff_id,
                        instance.from_uid,
                    ),
                );
            }
        }
    }
    inner_effects.push(ActEffectBuilder::magic_circle_delete(create_uid, circle_id));

    let cleanup = effect_container_step(0, 0, 0, inner_effects);
    let mut wrappers = vec![wrap_step(cleanup)];

    // Arrays that carry an `endSkills` slot fire it when the array
    // expires or is replaced. For Tuesday's "Horror Story Night"
    // (circle 22100003) the in-game description says the array
    // immediately resolves Poison on all enemies after ending; the
    // actual Poison settlement happens earlier in the round through
    // the standard DOT path, so LIVE only emits an empty SKILL
    // marker here (`et=162` wrapping a SKILL step whose `actId` is
    // the endSkills id and whose inner effects are empty).
    if let Some(end_skills_id) = end_skills_id {
        let target_uid = alive_enemy_uids.first().copied().unwrap_or(0);
        let end_marker = FightStepBuilder::skill(create_uid, target_uid, end_skills_id).build();
        wrappers.push(wrap_step(end_marker));
    }

    Some(build_effect_step(wrappers))
}
