use crate::network::packet::ClientPacket;
use crate::state::{ConnectionContext, handle_dungeon_end};
use crate::{error::AppError, state::send_end_fight_push};
use prost::Message;
use sonettobuf::{CmdId, EndDungeonReply, EndDungeonRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_dungeon_end_dungeon(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = EndDungeonRequest::decode(&req.data[..])?;

    let is_abort = request.is_abort.ok_or(AppError::InvalidRequest)?;

    tracing::info!("Dungeon ended with is_abort: {}", is_abort);

    let (fight_group, is_replay, chapter_id, episode_id, multiplication, player_id, pool, battle_id) = {
        let conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_ref()
            .ok_or(AppError::InvalidRequest)?;

        (
            battle.fight_group.clone(),
            battle.is_replay.unwrap_or(false),
            battle.chapter_id,
            battle.episode_id,
            battle.multiplication.unwrap_or(1),
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
            battle.fight_id.unwrap_or_default(),
        )
    };

    if is_abort {
        // Send end fight push for abort/loss
        send_end_fight_push(
            ctx.clone(),
            battle_id,
            0, // Loss/abort (0 = lose, 1 = win, 2 = turn_exhausted)
            fight_group.clone().unwrap_or_default(),
            vec![],
            vec![],
            !is_replay,
        )
        .await?;
    }

    // Handle dungeon end logic
    let is_victory = !is_abort;
    handle_dungeon_end(
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

    // Clear battle
    {
        let mut conn = ctx.lock().await;
        conn.active_battle = None;
    }

    let data = EndDungeonReply {};

    let mut conn = ctx.lock().await;
    conn.send_reply(CmdId::DungeonEndDungeonCmd, data, 0, req.up_tag)
        .await?;

    Ok(())
}
