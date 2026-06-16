pub(crate) mod executor;
pub(crate) mod ex_card;
mod op;
mod upgrade;
pub mod utils;

pub use op::CardOpType;
pub use upgrade::{apply_card_upgrades, skills_at_same_rank, upgrade_level1};
pub use utils::{is_ex_card};

pub fn skill_level(skill_id: i32, entities: &[sonettobuf::FightEntityInfo]) -> usize {
    entities.iter().flat_map(|e| [&e.skill_group1, &e.skill_group2]).find_map(|g| {
        g.iter().position(|&id| id == skill_id)
    }).unwrap_or(0)
}
