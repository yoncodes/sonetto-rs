use crate::network::packet::ClientPacket;
use crate::state::{ConnectionContext, handle_dungeon_end};
use crate::{error::AppError, state::send_end_fight_push};
use prost::Message;
use sonettobuf::{CmdId, EndFightReply, EndFightRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_fight_end_fight(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = EndFightRequest::decode(&req.data[..])?;

    let is_abort = request.is_abort.ok_or(AppError::InvalidRequest)?;

    tracing::info!("Fight ended with is_abort: {}", is_abort);

    let (fight_group, is_replay, battle_id, fight_data_mgr, chapter_id, episode_id, multiplication, player_id, pool) = {
        let conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_ref()
            .ok_or(AppError::InvalidRequest)?;

        (
            battle.fight_group.clone(),
            battle.is_replay.unwrap_or(false),
            battle.fight_id.unwrap_or_default(),
            battle.fight_data_mgr.clone(),
            battle.chapter_id,
            battle.episode_id,
            battle.multiplication.unwrap_or(1),
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    // Determine fight result: 0 = lose, 1 = win, 2 = turn_exhausted
    let result = if is_abort {
        0
    } else if let Some(mgr) = fight_data_mgr {
        mgr.check_battle_result()
    } else {
        1
    };

    tracing::info!("Fight ended with result: {} (0=lose, 1=win, 2=turn_exhausted)", result);

    // Send EndFightPush with proper result
    // TODO: Populate FightRecord with actual battle stats (damage, turns, etc.)
    send_end_fight_push(
        ctx.clone(),
        battle_id,
        result,
        fight_group.clone().unwrap_or_default(),
        vec![],     // TODO: Actual battle stats for fight_group_a
        vec![],     // TODO: Defender stats
        !is_replay,
    )
    .await?;

    // Handle dungeon end logic if this is a dungeon battle
    let is_victory = result == 1;
    let handled_dungeon = handle_dungeon_end(
        ctx.clone(),
        &pool,
        player_id,
        chapter_id,
        episode_id,
        &fight_group,
        multiplication,
        is_replay,
        is_victory,
    )
    .await?;

    // Clear battle if we handled dungeon logic
    if handled_dungeon {
        let mut conn = ctx.lock().await;
        conn.active_battle = None;
    }

    let data = EndFightReply {};

    let mut conn = ctx.lock().await;
    conn.send_reply(CmdId::FightEndFightCmd, data, 0, req.up_tag)
        .await?;
    Ok(())
}
