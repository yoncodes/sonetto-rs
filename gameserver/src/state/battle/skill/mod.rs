mod post_process;
pub(crate) mod precast;

pub mod cache;
pub mod classification;
pub mod classification_kind;
pub mod condition;
pub mod euphoria;
pub mod source_kind;

pub mod damage;
mod execution_guards;
pub(crate) mod executor;
mod phase;
pub mod sibling_coalesce;
pub mod targets;

pub(crate) use crate::state::battle::buff::apply_skill as buff;
pub(crate) use crate::state::battle::effect::behavior::random;
pub(crate) use crate::state::battle::effect::behavior::stats::{average_life, bloodlust, change_power};
pub(crate) use executor::{PendingMonsterChange, PendingSummon};
pub(crate) use precast::{
    collect_precast_skills_for_caster, infer_precast_per_decr_seed_cap,
};
pub use executor::{SkillExecutor, build_skill_act_effect};
pub use phase::{PhaseFilter, TriggerState};
pub use targets::get_entity;
