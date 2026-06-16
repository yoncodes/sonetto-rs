use crate::error::AppError;
use crate::send_push;
use crate::state::{generate_dungeon_rewards, ConnectionContext};
use crate::util::push::{send_dungeon_update_push, send_end_dungeon_push, send_red_dot_push};
use database::db::game::dungeons::{
    get_user_dungeon, should_update_dungeon_record, update_dungeon_progress,
};
use database::db::game::{dungeons::save_dungeon_record, equipment::build_equip_records};
use sonettobuf::{CmdId, FightGroup, InstructionDungeonInfoPush};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle dungeon completion logic (progress updates, rewards, pushes)
/// Returns true if dungeon logic was executed, false if not a dungeon
pub async fn handle_dungeon_end(
    ctx: Arc<Mutex<ConnectionContext>>,
    pool: &SqlitePool,
    player_id: i64,
    chapter_id: i32,
    episode_id: i32,
    fight_group: &Option<FightGroup>,
    multiplication: i32,
    is_replay: bool,
    is_victory: bool, // true = victory, false = loss/abort
) -> Result<bool, AppError> {
    // Check if this is a dungeon battle
    if chapter_id <= 0 || episode_id <= 0 {
        return Ok(false);
    }

    if is_victory && !is_replay {
        // Update player's dungeon progress
        let stars_earned = 2; // TODO: Calculate based on performance
        update_dungeon_progress(pool, player_id, chapter_id, episode_id, stars_earned).await?;

        // TODO: Get actual round count from battle state
        let record_round = 1;
        let should_save_record =
            should_update_dungeon_record(pool, player_id, episode_id, record_round, fight_group)
                .await?;

        if should_save_record {
            let equips = build_equip_records(pool, player_id, fight_group).await?;
            save_dungeon_record(
                pool,
                player_id,
                episode_id,
                record_round,
                &fight_group.clone().unwrap_or_default(),
                equips,
            )
            .await?;
        }

        tracing::info!(
            "Dungeon completed: episode={}, round={}, record_saved={}",
            episode_id,
            record_round,
            should_save_record
        );
    }

    if is_victory {
        // Send dungeon completion pushes for victory
        send_push!(
            ctx,
            CmdId::DungeonInstructionDungeonInfoPushCmd,
            InstructionDungeonInfoPush,
            "dungeon/instruction_dungeon_info.json"
        );

        let updated_dungeon = get_user_dungeon(pool, player_id, chapter_id, episode_id).await?;

        let game_data = config::configs::get();
        let chapter_type = game_data
            .chapter
            .iter()
            .find(|c| c.id == chapter_id)
            .map(|c| c.r#type)
            .unwrap_or(6);

        send_dungeon_update_push(
            ctx.clone(),
            chapter_id,
            episode_id,
            updated_dungeon.star,
            updated_dungeon.challenge_count,
            updated_dungeon.has_record,
            chapter_type,
            2, // TODO: Calculate today's chapter completions
            2, // TODO: Calculate today's chapter attempts
        )
        .await?;

        // Generate and send rewards
        let is_first_clear = updated_dungeon.challenge_count == 1;
        let rewards = generate_dungeon_rewards(episode_id, is_first_clear, multiplication);

        let mut all_rewards = rewards.normal_bonus.clone();
        all_rewards.extend(rewards.first_bonus);
        all_rewards.extend(rewards.free_bonus);

        send_end_dungeon_push(ctx.clone(), chapter_id, episode_id, all_rewards).await?;

        send_red_dot_push(Arc::clone(&ctx), player_id, Some(vec![1027, 1047])).await?;
    } else {
        // Send empty end dungeon push for loss/abort
        send_end_dungeon_push(ctx.clone(), chapter_id, episode_id, vec![]).await?;
    }

    Ok(true)
}
