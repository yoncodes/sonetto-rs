use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use sonettobuf::{CmdId, GetFightCardDeckInfoReply};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_get_fight_card_deck_info(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let mut conn = ctx.lock().await;
    let deck_infos = conn.active_battle.as_ref()
        .and_then(|b| b.fight_data_mgr.as_ref())
        .map(|m| m.managers.deck_mgr.player_deck.clone())
        .unwrap_or_default();
    conn.send_reply(CmdId::GetFightCardDeckInfoCmd, GetFightCardDeckInfoReply { deck_infos }, 0, req.up_tag).await?;
    Ok(())
}
