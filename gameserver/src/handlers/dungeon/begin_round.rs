use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use database::db::game::battle::save_round_operations;
use prost::Message;
use sonettobuf::{BeginRoundReply, BeginRoundRequest, CmdId};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_begin_round(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = BeginRoundRequest::decode(&req.data[..])?;

    let (is_replay, battle_id, episode_id, round_num, mut fight_data_mgr) = {
        let mut conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_mut()
            .ok_or(AppError::InvalidRequest)?;
        let mgr = battle
            .fight_data_mgr
            .take()
            .ok_or(AppError::InvalidRequest)?;
        (
            battle.is_replay.unwrap_or(false),
            battle.fight_id.unwrap_or_default(),
            battle.episode_id,
            mgr.fight().cur_round.unwrap_or(1),
            mgr,
        )
    };

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    let round = fight_data_mgr
        .process_round(request.opers.clone(), None)
        .await?;
    {
        let mut conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_mut()
            .ok_or(AppError::InvalidRequest)?;
        fight_data_mgr.last_round = Some(round.clone());
        battle.fight_data_mgr = Some(fight_data_mgr);
    }

    let mut conn = ctx.lock().await;
    conn.send_reply(
        CmdId::BeginRoundCmd,
        BeginRoundReply {
            round: Some(round.clone()),
        },
        0,
        req.up_tag,
    )
    .await?;

    if !is_replay {
        save_round_operations(
            &pool,
            player_id,
            episode_id,
            battle_id,
            round_num,
            vec![],
            request.opers,
        )
        .await?;
    }

    Ok(())
}
