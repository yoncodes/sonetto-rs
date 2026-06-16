use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use prost::Message;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::{CmdId, UseClothSkillReply, UseClothSkillRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_use_cloth_skill(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = UseClothSkillRequest::decode(&req.data[..])?;
    let skill_id = request.skill_id.unwrap_or(0);

    let mut fdm = {
        let mut conn = ctx.lock().await;
        let battle = conn.active_battle.as_mut().ok_or(AppError::InvalidRequest)?;
        battle.fight_data_mgr.take().ok_or(AppError::InvalidRequest)?
    };

    let mut rng = StdRng::from_entropy();
    let round = fdm.execute_cloth_skill(skill_id, &mut rng).map_err(|_| AppError::InvalidRequest)?;

    {
        let mut conn = ctx.lock().await;
        let battle = conn.active_battle.as_mut().ok_or(AppError::InvalidRequest)?;
        battle.fight_data_mgr = Some(fdm);
    }

    let reply = UseClothSkillReply { round: Some(round) };
    ctx.lock().await.send_reply(CmdId::UseClothSkillCmd, reply, 0, req.up_tag).await?;

    Ok(())
}
