//! Player-actions phase: walk the operations the client recorded
//! for this round, apply each one, and shape the resulting steps.
//!
//! For each player operation:
//! 1. Compute the cloth-power delta and ex-gain prelude before the
//!    card resolves.
//! 2. Execute the operation and apply the resulting step to managers.
//! 3. Run `expand_trigger_chain` and either embed the trigger output
//!    into the host SKILL wrapper's nested-skill child or splice it
//!    at the host's trigger insert index.
//! 4. Append `MonitorContinueChannel` reactives surfaced from the
//!    player's buffs.
//! 5. Inject any boss-side `BeAttacked` reactive on the host's damaged
//!    enemies.
//! 6. Inject card-host injury markers for the player carrying an
//!    injury counter.
//!
//! Mirrors `phase/enemy_actions.rs` but threaded for player-side
//! semantics (cloth power, ex gain, injury markers, channel
//! reactives).

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::event::{Event, apply::events_to_steps};
use crate::state::battle::{
    card::executor::play_card,
    deck::{DeckManager, make_card},
    context::FightContext,
    event_queue::{
        BattleEvent, HostEventAccumulator, HostLane, HostSide, check_host_lane_membership,
        register_round_host,
    },
    manager::round_mgr::{apply_step_and_maybe_sync, check_battle_end, deleted_buff_ids_from_delta, expand_trigger_chain, inject_be_attacked_reactives_onto_player_host},
    mechanics::{channel as channel_mechanics, injury_counter, magic_circle},
    passives::collector::CollectedPassives,
    round::RoundState,
    skill::SkillExecutor,
    step_walker,
    steps::{ex_gain, trigger_embed},
    trigger::passes::sync_blood_value_baseline,
};

#[derive(Debug, Clone, Copy)]
enum HostAccumulatorLane {
    TriggerLane,
}

fn push_host_accumulator_lane(
    accumulator: &mut HostEventAccumulator,
    lane: HostAccumulatorLane,
    effect: ActEffect,
) {
    let event = BattleEvent::SerializedActEffect { effect };
    match lane {
        HostAccumulatorLane::TriggerLane => accumulator.push_trigger_lane(event),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    executor: &mut SkillExecutor,
    state: &mut RoundState,
    deck_mgr: &mut DeckManager,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
    sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
    for evt in std::mem::take(&mut state.player_events) {
        let oper = match &evt {
            Event::CardPlayed { card: _, oper } => {
                steps.extend(events_to_steps(ctx.on_use_card(&evt)));
                oper.clone()
            }
            Event::CardMoved { .. } => {
                steps.extend(events_to_steps(ctx.on_move_card(&evt)));
                continue;
            }
            Event::CardUpgrade { .. } => {
                steps.extend(events_to_steps(ctx.on_card_upgrade(&evt)));
                continue;
            }
            Event::SimulateDissolveCard { oper } => {
                let dissolve_index = (oper.param1.unwrap_or(1) - 1) as usize;
                if dissolve_index < state.selected_cards.len() {
                    state.selected_cards.remove(dissolve_index);
                }
                steps.push(FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    act_effect: vec![crate::state::battle::fight_step::ActEffectBuilder::cards_push(state.selected_cards.clone(), Some(1))],
                    ..Default::default()
                });
                continue;
            }
            _ => continue,
        };

        // Only PLAY_CARD/AssistBoss/PlayerFinisherSkill/BloodPool continue
        ctx.managers.buff_mgr.clear_step_deleted_buff_ids();
        let ex_step_after_op = ex_gain::pre_operation_ex_gain(ctx, state, &oper);
        let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
        let step = play_card(executor, rng, ctx, state, oper).await?;
        if step.act_type.unwrap_or(0) == 0 {
            continue;
        }

        apply_step_and_maybe_sync(ctx, &step, true)?;
        
        let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
        let runtime_deleted_buff_ids =
            deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

        let is_player_skill = step.act_type == Some(fight_step::ActType::Skill as i32)
            && step.from_id.unwrap_or(0) >= 0;
        if !is_player_skill {
            let expanded_steps =
                expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
            steps.extend(expanded_steps);
            state.is_finish = check_battle_end(ctx.fight);
            if state.is_finish {
                break;
            }
            continue;
        }

        let suppress_pre_op_ex =
            ex_gain::skill_suppresses_pre_operation_ex(step.act_id.unwrap_or(0));
        // Generate EX cards for heroes that have reached max EX points
        if let Some(attacker) = ctx.fight.attacker.as_ref() {
            let entities: Vec<_> = attacker.entitys.iter().chain(attacker.sub_entitys.iter())
                .filter_map(|e| {
                    let uid = e.uid.unwrap_or(0);
                    if uid > 0
                        && ctx.managers.entity_mgr.get_ex_point(uid) >= ctx.managers.entity_mgr.get_ex_max(uid)
                        && ctx.managers.entity_mgr.get_ex_max(uid) > 0
                    {
                        Some((uid, e.ex_skill, e.model_id))
                    } else {
                        None
                    }
                })
                .collect();
            for (uid, ex_skill, hero_id) in entities {
                if let (Some(ex_skill), Some(hero_id)) = (ex_skill, hero_id) {
                    if ex_skill != 0
                        && !deck_mgr.player_ex_deck.iter().any(|c| c.uid == Some(uid) && c.skill_id == Some(ex_skill))
                        && !deck_mgr.player_hand.iter().any(|c| c.uid == Some(uid) && c.skill_id == Some(ex_skill)) {
                        tracing::info!("ex_deck: add uid={} ex_skill={}", uid, ex_skill);
                        deck_mgr.player_ex_deck.push(make_card(hero_id, ex_skill, uid, false));
                    }
                }
            }
        }
        if !suppress_pre_op_ex && let Some(ex_step) = ex_step_after_op.clone() {
            steps.push(ex_step);
        }
        let mut host_step = step.clone();
        step_walker::inline_magic_circle_root_wrapper(&mut host_step);
        let mut accumulator = HostEventAccumulator::new();
        for effect in host_step.act_effect.clone() {
            accumulator.push_direct(BattleEvent::SerializedActEffect { effect });
        }
        let mc_kind = magic_circle::apply_magic_circle_self_skill_embeds_with_accumulator(
            ctx,
            &mut host_step,
            &mut accumulator,
        );
        let expanded_steps =
            expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
        // Splice combat triggers as direct children of the host wrapper.
        // LIVE always attaches reactive passives at depth=1 under the host
        // skill wrapper — verified across battle1/2/3 fixtures (every player
        // skill cast has triggers as flat children, no `host_act_id - 20`
        // pre-embed pattern).
        //
        // The earlier branch here used `host_step.act_id - 20` as a
        // `preferred_nested_act_id` lookup with an `or_else(rposition)`
        // fallback that picked ANY non-host SKILL fightStep when the
        // preferred id was missing. The `-20` was a brittle heuristic from
        // the original `76ca5690 feat(battle): achieve live pcap parity`
        // commit that doesn't match any host in current fixtures (0/40
        // player skill casts across battle1+2+3), and the rposition
        // fallback caused triggers to be wrongly embedded inside the first
        // emitted self-passive (e.g. Sotheby's `1148002` battle-rule
        // wrapper) — Willow's `31040141` reactive ended up at depth=2
        // nested inside `1148002` instead of as a direct child of
        // `30090111` at depth=1.
        let trigger_offset = accumulator.lane_iter(HostLane::Trigger).count();
        for trigger_step in expanded_steps.into_iter().skip(1) {
            let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
            push_host_accumulator_lane(
                &mut accumulator,
                HostAccumulatorLane::TriggerLane,
                embedded,
            );
        }
        accumulator.splice_lane_drain_into_host(
            HostLane::Trigger,
            trigger_offset,
            &mut host_step,
            |host| step_walker::host_trigger_insert_index(host),
        );
        let monitor_embeds =
            channel_mechanics::build_monitor_continue_channel_embeds(ctx, &step, &host_step);
        let monitor_offset = accumulator.lane_iter(HostLane::Trigger).count();
        for effect in monitor_embeds {
            push_host_accumulator_lane(&mut accumulator, HostAccumulatorLane::TriggerLane, effect);
        }
        accumulator.splice_lane_drain_into_host(
            HostLane::Trigger,
            monitor_offset,
            &mut host_step,
            |host| step_walker::host_trigger_insert_index(host),
        );
        host_step.act_effect = trigger_embed::flatten_self_nested_skill_effects_v(
            host_step.act_type,
            host_step.act_id,
            host_step.from_id,
            std::mem::take(&mut host_step.act_effect),
        );
        host_step.act_effect = trigger_embed::normalize_player_skill_effect_order_v(
            host_step.act_type,
            std::mem::take(&mut host_step.act_effect),
        );
        let be_attacked_offset = accumulator.lane_iter(HostLane::BeAttacked).count();
        let be_attacked_insert_at = inject_be_attacked_reactives_onto_player_host(
            state,
            &host_step,
            ctx,
            &mut accumulator,
        );
        if let Some(insert_at) = be_attacked_insert_at {
            accumulator.splice_lane_drain_into_host(
                HostLane::BeAttacked,
                be_attacked_offset,
                &mut host_step,
                |_| insert_at,
            );
        }
        if let Some((holder_uid, injury_count)) =
            injury_counter::find_card_host_injury_marker_params(
                ctx.fight,
                host_step.from_id.unwrap_or(0),
            )
        {
            let mut markers = injury_counter::inject_card_host_injury_markers(
                &host_step,
                ctx.fight,
                holder_uid,
                injury_count,
                &mut accumulator,
            );
            // Splice in reverse-index order so earlier positions don't shift.
            markers.sort_by(|(a, _), (b, _)| b.cmp(a));
            for (idx, marker) in markers {
                host_step.act_effect.insert(idx, marker);
            }
        }
        let (direct, trigger, be_attacked, injury) = accumulator.lane_counts();
        tracing::debug!(
            target: "phase5_accumulator",
            "host_step.act_effect.len()={} acc.total={} (direct={} trigger={} be_attacked={} injury={}) skill_id={} caster={}",
            host_step.act_effect.len(),
            accumulator.child_count(),
            direct,
            trigger,
            be_attacked,
            injury,
            host_step.act_id.unwrap_or(0),
            host_step.from_id.unwrap_or(0),
        );
        let mc_skipped = matches!(mc_kind, magic_circle::MagicCircleApplyKind::NestedPath);
        if !mc_skipped {
            let mut captured_effects: Vec<&ActEffect> = Vec::new();
            for effect in accumulator.iter_captured_act_effects() {
                if let Some(nested_step) = effect.fight_step.as_ref()
                    && nested_step.act_type == Some(fight_step::ActType::Skill as i32)
                    && nested_step.act_id == host_step.act_id
                {
                    captured_effects.extend(nested_step.act_effect.iter());
                } else {
                    captured_effects.push(effect);
                }
            }
            match check_host_lane_membership(&captured_effects, &host_step.act_effect) {
                Ok(()) => {}
                Err(diff) => {
                    for (lane_name, lane) in [
                        ("direct", HostLane::Direct),
                        ("trigger", HostLane::Trigger),
                        ("be_attacked", HostLane::BeAttacked),
                        ("injury", HostLane::Injury),
                    ] {
                        let lane_count = accumulator.lane_iter(lane).count();
                        tracing::debug!(
                            target: "phase5_membership",
                            "lane={} count={} skill_id={} caster={}",
                            lane_name,
                            lane_count,
                            host_step.act_id.unwrap_or(0),
                            host_step.from_id.unwrap_or(0),
                        );
                    }
                    tracing::warn!(
                        target: "phase5_membership",
                        "lane membership diff: {} captured effects missing from host (skill_id={} caster={})",
                        diff.missing_from_host.len(),
                        host_step.act_id.unwrap_or(0),
                        host_step.from_id.unwrap_or(0),
                    );
                    debug_assert!(
                        diff.missing_from_host.is_empty(),
                        "Phase 5 lane membership assertion: {} captured effects missing from host (skill_id={})",
                        diff.missing_from_host.len(),
                        host_step.act_id.unwrap_or(0),
                    );
                }
            }
        }
        register_round_host(
            host_step.from_id.unwrap_or(0),
            host_step.act_id.unwrap_or(0),
            HostSide::Player,
            steps.len(),
        );
        steps.push(host_step);
        state.is_finish = check_battle_end(ctx.fight);
        if state.is_finish {
            break;
        }
    }

    state.before_cards2 = deck_mgr.player_hand.clone();
    let (cards2, upgrades2) = deck_mgr.refill_player_hand(rng, 0, ctx.fight, &ctx.managers.entity_mgr);
    for _ in 0..upgrades2 {
        let evt = crate::state::battle::event::Event::CardUpgrade { card: sonettobuf::CardInfo::default() };
        steps.extend(events_to_steps(ctx.on_card_upgrade(&evt)));
    }
    state.team_a_cards2 = cards2;

    Ok(())
}
