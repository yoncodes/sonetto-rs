use super::super::BattleContext;
use super::attacker::Attacker;
use super::defender::Defender;
use anyhow::Result;
use sonettobuf::{Fight, FightGroup, FightTaskBox, fight::FightActType};
use sqlx::SqlitePool;

pub struct BuiltFight {
    pub fight: Fight,
}

pub async fn build_fight(
    pool: &SqlitePool,
    ctx: &BattleContext,
    fight_group: &FightGroup,
) -> Result<BuiltFight> {
    let attacker = Attacker::get(pool, ctx.player_id, fight_group).await?;
    let defender = Defender::get(ctx.episode_id).await?;

    let mut fight = Fight {
        attacker: Some(attacker),
        defender: Some(defender.team),
        cur_round: Some(1),
        max_round: Some(defender.max_round),
        is_finish: Some(false),
        cur_wave: Some(1),
        battle_id: Some(ctx.battle_id),
        magic_circle: None,
        version: Some(5),
        is_record: Some(false),
        episode_id: Some(ctx.episode_id),
        fight_act_type: Some(FightActType::Normal.into()),
        last_change_hero_uid: Some(0),
        progress: Some(0),
        progress_max: Some(0),
        param: vec![],
        custom_data: vec![],
        fight_task_box: Some(FightTaskBox { tasks: vec![] }),
        progress_list: vec![],
    };

    //Use rule_mgr to manage battle passives
    //let passives = collect_battle_passives(ctx.battle_id);
    //inject_battle_passives(&mut fight, &passives);

    Ok(BuiltFight { fight })
}
