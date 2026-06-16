use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::{
    ActiveBattle, BattleContext, ConnectionContext, create_battle, default_max_ap,
};
use config::configs;
use database::db::game::dungeons::{get_user_dungeon, update_dungeon_progress};
use prost::Message;
use sonettobuf::{CmdId, DungeonUpdatePush, StartDungeonReply, StartDungeonRequest, UserDungeon};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_start_dungeon(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = StartDungeonRequest::decode(&req.data[..])?;
    tracing::info!("Received start dungeon request {:?}", request);

    let chapter_id = request.chapter_id.unwrap_or(0);
    let episode_id = request.episode_id.unwrap_or(0);
    let use_record = request.use_record.unwrap_or(false);
    let multiplication = request.multiplication.unwrap_or(1);

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    let game_data = configs::get();

    let episode_cfg = game_data
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .ok_or(AppError::InvalidRequest)?;

    if episode_cfg.battle_id == 0 {
        return handle_story_only_episode(ctx, req, chapter_id, episode_id).await;
    }

    let fight_group = request.fight_group.ok_or(AppError::InvalidRequest)?;

    let hero_count = fight_group.hero_list.iter().filter(|&&u| u != 0).count();

    let battle_id = episode_cfg.battle_id;
    let max_ap = default_max_ap(episode_id, hero_count);

    let battle_ctx = BattleContext {
        player_id,
        chapter_id,
        episode_id,
        battle_id,
        max_ap,
    };

    let seed = (player_id as u64) ^ (episode_id as u64) ^ 0xA11C;
    let (initial_round, mut fight_data_mgr) =
        create_battle(&pool, battle_ctx, &fight_group, seed).await?;

    let mut card_push = fight_data_mgr.initial_card_push.take().unwrap_or_default();
    card_push.card_group = fight_data_mgr.managers.deck_mgr.player_hand.clone();

    let fight_snapshot = fight_data_mgr.pre_fight.clone()
        .unwrap_or_else(|| fight_data_mgr.fight().clone());

    {
        let mut conn: tokio::sync::MutexGuard<'_, ConnectionContext> = ctx.lock().await;
        conn.active_battle = Some(ActiveBattle {
            tower_type: None,
            tower_id: None,
            layer_id: None,
            episode_id,
            chapter_id,
            difficulty: None,
            talent_plan_id: None,
            fight_group: Some(fight_group.clone()),
            is_replay: Some(use_record),
            replay_episode_id: Some(episode_id),
            fight_id: Some(chrono::Utc::now().timestamp_millis()),
            multiplication: Some(multiplication),
            fight_data_mgr: Some(fight_data_mgr),
        });
    }

    /*  let updated_dungeon = get_user_dungeon(&pool, player_id, chapter_id, episode_id).await?;

    let chapter_type = game_data
        .chapter
        .iter()
        .find(|c| c.id == chapter_id)
        .map(|c| c.r#type)
        .unwrap_or(6);

    let chapter_type_nums = vec![sonettobuf::UserChapterTypeNum {
        chapter_type: Some(chapter_type),
        today_pass_num: Some(1),
        today_total_num: Some(2),
    }];

    let dungeon_push = DungeonUpdatePush {
        dungeon_info: Some(UserDungeon {
            chapter_id: Some(chapter_id),
            episode_id: Some(episode_id),
            star: Some(updated_dungeon.star),
            challenge_count: Some(updated_dungeon.challenge_count),
            has_record: Some(updated_dungeon.has_record),
            left_return_all_num: Some(1),
            today_pass_num: Some(0),
            today_total_num: Some(0),
        }),
        chapter_type_nums,
    };*/

    let reply = StartDungeonReply {
        fight: Some(fight_snapshot),
        round: Some(initial_round),
    };

    let mut conn = ctx.lock().await;

    // weird this doesn't belong here lol

    //conn.notify(CmdId::DungeonUpdatePushCmd, dungeon_push).await?;
    //

    tracing::warn!(
        "reply round ex_point_info[0] current_hp={:?}",
        reply
            .round
            .as_ref()
            .and_then(|r| r.ex_point_info.first())
            .and_then(|e| e.current_hp)
    );

    conn.send_reply(CmdId::StartDungeonCmd, reply, 0, req.up_tag)
        .await?;

    // This needs to be sent after the reply else client bugs out
    conn.notify(CmdId::CardInfoPushCmd, card_push).await?;

    Ok(())
}

async fn handle_story_only_episode(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
    chapter_id: i32,
    episode_id: i32,
) -> Result<(), AppError> {
    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    // Mark as completed with 1 star since it's story-only
    update_dungeon_progress(&pool, player_id, chapter_id, episode_id, 1).await?;

    // Fetch the updated record
    let updated_dungeon = get_user_dungeon(&pool, player_id, chapter_id, episode_id).await?;

    let dungeon_push = DungeonUpdatePush {
        dungeon_info: Some(UserDungeon {
            chapter_id: Some(chapter_id),
            episode_id: Some(episode_id),
            star: Some(updated_dungeon.star),
            challenge_count: Some(updated_dungeon.challenge_count),
            has_record: Some(updated_dungeon.has_record),
            left_return_all_num: Some(1),
            today_pass_num: Some(0),
            today_total_num: Some(0),
        }),
        chapter_type_nums: vec![],
    };

    let reply = StartDungeonReply {
        fight: None,
        round: None,
    };

    let mut conn = ctx.lock().await;

    conn.notify(CmdId::DungeonUpdatePushCmd, dungeon_push)
        .await?;

    conn.send_reply(CmdId::StartDungeonCmd, reply, 0, req.up_tag)
        .await?;

    Ok(())
}
