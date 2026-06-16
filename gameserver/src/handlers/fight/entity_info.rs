use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use sonettobuf::{CmdId, EntityInfoReply, EntityInfoRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_entity_info(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request: EntityInfoRequest = req.decode_message()?;

    let conn = ctx.lock().await;
    let entity_info = request.uid.and_then(|uid| {
        let battle = conn.active_battle.as_ref()?;
        let fight = battle.fight_data_mgr.as_ref()?.fight();
        fight.attacker.as_ref()
            .and_then(|team| team.entitys.iter().find(|e| e.uid == Some(uid)))
            .or_else(|| {
                fight.defender.as_ref()
                    .and_then(|team| team.entitys.iter().find(|e| e.uid == Some(uid)))
            })
            .cloned()
    });

    drop(conn);
    let mut conn = ctx.lock().await;
    conn.send_reply(CmdId::EntityInfoCmd, EntityInfoReply { entity_info }, 0, req.up_tag)
        .await?;
    Ok(())
}
