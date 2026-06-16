use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::{
    ActiveBattle, BattleContext, ConnectionContext, create_battle, default_max_ap,
};
use config::configs;
use prost::Message;
use sonettobuf::{
    CmdId, DungeonUpdatePush, StartDungeonReply, StartTowerBattleReply, StartTowerBattleRequest,
    UserDungeon,
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_start_tower_battle(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = StartTowerBattleRequest::decode(&req.data[..])?;

    let start_req = request
        .start_dungeon_request
        .ok_or(AppError::InvalidRequest)?;
    let fight_group = start_req.fight_group.ok_or(AppError::InvalidRequest)?;

    let dungeon_type = request.r#type.ok_or(AppError::InvalidRequest)?;
    let tower_id = request.tower_id.ok_or(AppError::InvalidRequest)?;
    let layer_id = request.layer_id.ok_or(AppError::InvalidRequest)?;
    let difficulty = request.difficulty.ok_or(AppError::InvalidRequest)?;
    let talent_plan_id = request.talent_plan_id.unwrap_or(0);

    let chapter_id = start_req.chapter_id.unwrap_or(0);
    let episode_id = start_req.episode_id.unwrap_or(0);

    tracing::info!(
        "Start tower battle: type={}, tower={}, layer={}, episode={}, diff={}, talent={}",
        dungeon_type,
        tower_id,
        layer_id,
        episode_id,
        difficulty,
        talent_plan_id
    );

    tracing::info!(
        "Fight group: heroes={:?}, cloth={}, assist_boss={}",
        fight_group.hero_list,
        fight_group.cloth_id.unwrap_or(1),
        fight_group.assist_boss_id.unwrap_or(0)
    );

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    let hero_count = fight_group.hero_list.iter().filter(|&&u| u != 0).count();

    let game_data = configs::get();
    let battle_id = game_data
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .ok_or(AppError::InvalidRequest)?
        .battle_id;

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
        let mut conn = ctx.lock().await;
        conn.active_battle = Some(ActiveBattle {
            tower_type: Some(dungeon_type),
            tower_id: Some(tower_id),
            layer_id: Some(layer_id),
            episode_id,
            chapter_id,
            difficulty: Some(difficulty),
            talent_plan_id: Some(talent_plan_id),
            fight_group: Some(fight_group.clone()),
            is_replay: None,
            replay_episode_id: None,
            fight_id: Some(chrono::Utc::now().timestamp_millis()),
            multiplication: None,
            fight_data_mgr: Some(fight_data_mgr),
        });
    }

    let start_reply = StartTowerBattleReply {
        start_dungeon_reply: Some(StartDungeonReply {
            fight: Some(fight_snapshot),
            round: Some(initial_round),
        }),
        r#type: Some(dungeon_type),
        tower_id: Some(tower_id),
        layer_id: Some(layer_id),
        difficulty: Some(difficulty),
        talent_plan_id: Some(talent_plan_id),
    };

    let dungeon_push = DungeonUpdatePush {
        dungeon_info: Some(UserDungeon {
            chapter_id: Some(chapter_id),
            episode_id: Some(episode_id),
            star: Some(0),
            challenge_count: Some(0),
            has_record: Some(false),
            left_return_all_num: Some(1),
            today_pass_num: Some(0),
            today_total_num: Some(0),
        }),
        chapter_type_nums: vec![],
    };

    let mut conn = ctx.lock().await;

    conn.notify(CmdId::DungeonUpdatePushCmd, dungeon_push)
        .await?;

    conn.send_reply(CmdId::StartTowerBattleCmd, start_reply, 0, req.up_tag)
        .await?;

    {
        let mut conn = ctx.lock().await;
        conn.notify(CmdId::CardInfoPushCmd, card_push).await?;
    }

    Ok(())
}
