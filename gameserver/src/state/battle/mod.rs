mod auto;
pub(crate) mod ai;
mod card;
pub mod cloth;
pub mod deck;
pub(crate) mod operation;
mod passives;

pub mod context;
pub mod destiny;
pub mod effect;
pub mod dungeon_end_logic;
pub mod emission_timeline;
pub mod end_fight;
pub mod equipment;
pub mod event_queue;
pub mod event;
pub mod types;

pub mod manager;
pub mod mechanics;
pub mod phase;
pub mod rewards;
pub mod round;
pub mod round_end_emission;
pub mod round_state;
pub mod rule;
pub mod step_walker;
pub mod steps;

pub mod utils;

pub mod buff;
pub mod buff_actions;
pub mod entity;
pub mod fight;
pub mod fight_step;
pub mod hero;
pub mod heroes;
pub mod skill;
pub mod trigger;

use anyhow::Result;
use sonettobuf::FightRound;
use sqlx::SqlitePool;

pub use auto::generate_auto_opers;
pub use deck::default_max_ap;
pub use types::{behavior::BehaviorType, condition::ConditionType};

use crate::state::battle::manager::fight_data_mgr::FightDataMgr;

#[allow(dead_code)]
pub struct BattleContext {
    pub player_id: i64,
    pub chapter_id: i32,
    pub episode_id: i32,
    pub battle_id: i32,
    pub max_ap: i32,
}

pub async fn create_battle(
    pool: &SqlitePool,
    ctx: BattleContext,
    fight_group: &sonettobuf::FightGroup,
    seed: u64,
) -> Result<(FightRound, FightDataMgr)> {
    let built_fight = fight::builder::build_fight(pool, &ctx, fight_group).await?;
    let mut fight_data_mgr = FightDataMgr::new(built_fight.fight, ctx.max_ap);
    let initial_round = round::build_initial_round(&mut fight_data_mgr, ctx.battle_id, seed)?;
    Ok((initial_round, fight_data_mgr))
}
