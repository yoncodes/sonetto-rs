use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{BeginRoundOper, CardInfo, Fight, FightRound, FightStep};

use crate::state::battle::{
    context::RoundContext,
    emission_timeline::EmissionTimeline,
    event_queue::{find_attachment_candidates, AttachmentResolver},
    fight_step::{ActEffectBuilder, FightStepBuilder},
    heroes::pickles,
    manager::round_mgr::{apply_step_and_maybe_sync, sync_new_change_wave_snapshot},
    mechanics,
    phase,
    round_end_emission,
    skill::SkillExecutor,
};

#[allow(clippy::too_many_arguments)]
pub async fn process_round(
    rng: &mut StdRng,
    round_ctx: &mut RoundContext<'_, '_>,
    executor: &mut SkillExecutor,
    operations: Vec<BeginRoundOper>,
    ai_override_steps: Option<Vec<FightStep>>,
    replay_selected_cards: Option<Vec<CardInfo>>,
    replay_silent_ops: Option<Vec<bool>>,
    replay_wave_snapshots: Option<&[Fight]>,
) -> Result<FightRound> {
    let mut deck_mgr = std::mem::take(&mut round_ctx.fight_ctx.managers.deck_mgr);
    round_ctx.fight_ctx.managers.cloth_mgr.reset();

    let replay_wave_snapshots = replay_wave_snapshots.unwrap_or(&[]);
    let replay_wave_snapshot_applied = !replay_wave_snapshots.is_empty();
    let replay_wave_snapshot_target_wave = replay_wave_snapshots
        .iter()
        .filter_map(|snapshot| snapshot.cur_wave)
        .last();
    let ctx = &mut *round_ctx.fight_ctx;
    if replay_wave_snapshot_applied {
        for snapshot in replay_wave_snapshots {
            sync_new_change_wave_snapshot(ctx, snapshot);
        }
    }

    let mut open = phase::round_open::run(
        round_ctx,
        rng,
        &mut deck_mgr,
        ai_override_steps.as_deref(),
        &operations,
        replay_selected_cards.as_deref(),
        replay_silent_ops.as_deref(),
    );
    open.state.replay_wave_snapshot_applied = replay_wave_snapshot_applied;
    open.state.replay_wave_snapshot_target_wave = replay_wave_snapshot_target_wave;
    open.state.ai_use_cards = std::mem::take(&mut deck_mgr.next_ai_use_cards);
    let ctx = &mut *round_ctx.fight_ctx;
    if replay_wave_snapshot_applied {
        for snapshot in replay_wave_snapshots {
            open.steps.push(
                FightStepBuilder::effect()
                    .with(ActEffectBuilder::new_change_wave(snapshot.clone()))
                    .build(),
            );
        }
    }

    for step in mechanics::dot_settle_round_start::build_round_start_dot_settle_steps(ctx) {
        apply_step_and_maybe_sync(ctx, &step, true)?;
        open.steps.push(step);
    }

    phase::player_actions::run(
        rng,
        ctx,
        executor,
        &mut open.state,
        &mut deck_mgr,
        &open.collected,
        &mut open.steps,
    )
    .await?;

    phase::non_terminal_round::run(
        rng,
        ctx,
        executor,
        &mut open.state,
        open.selected_for_round_end.clone(),
        &open.collected,
        open.defender_uid_checkpoint,
        &mut deck_mgr,
        &mut open.steps,
    )
    .await?;

    round_end_emission::merge_post_turn_reactives_into_host(&mut open.steps);
    mechanics::nautika::strip_duplicate_change_round_markers(&mut open.steps);
    mechanics::nautika::consolidate_into_bundle(ctx.fight, &mut open.steps);
    mechanics::nautika::strip_redundant_post_round_emissions(ctx.fight, &mut open.steps);
    round_end_emission::coalesce_late_tail_exclude_battle_rule_passives(&mut open.steps);
    if round_end_emission::repair_boss_state_cycle_second_wave(ctx, &mut open.steps) {
        ctx.sync();
    }

    let attachment_candidates = find_attachment_candidates(&open.steps);
    AttachmentResolver::apply(&mut open.steps, attachment_candidates);
    if pickles::repair_round_end_hedonism_emission(
        ctx.fight,
        &mut ctx.managers.buff_mgr,
        &mut open.steps,
    ) {
        ctx.sync();
    }
    if round_end_emission::repair_rubuska_round_end_heal_markers(ctx, &mut open.steps) {
        ctx.sync();
    }

    if EmissionTimeline::dump_enabled() {
        eprint!("{}", ctx.mechanics.emission_timeline.dump());
    }

    let result = phase::build_round_output::build_round_output(round_ctx, open, &mut deck_mgr, rng);
    round_ctx.fight_ctx.managers.deck_mgr = deck_mgr;
    result
}
