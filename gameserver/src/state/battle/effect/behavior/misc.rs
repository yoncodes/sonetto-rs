use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{PendingMonsterChange, PendingSummon, SkillExecutor},
};
use rand::rngs::StdRng;
use sonettobuf::Fight;

/// Misc action — handler for the behavior variants that are
/// currently no-ops or simple skill-execution placeholders. Each
/// variant either emits nothing or logs a warning.
///
/// Variants owned:
/// * `Summon { .. }` — queues a silent defender-side entity spawn.
/// * `MonsterChange { .. }` — applies entity form swap via
///   `mechanics::phase_change::transform_entity`.
/// * `Kill`, `ShellUseSkill { .. }`, `ShellAssign { .. }`,
///   `BeAttackedAssassinate { .. }`, `CrystalAddCard` — placeholders.
/// * `IgnoreSkillConfigDamageRate` — flag-only behavior; suppression
///   happens elsewhere in the executor.
/// * `Unknown { raw }` — log and skip.
pub fn execute(
    _fight: &Fight,
    _managers: &mut Managers,
    _mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    caster_uid: i64,
    targets: Vec<i64>,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_60008Summon
        | BehaviourType::_60013SummonSp
        | BehaviourType::_60056SummonSp2 => {
            let monster_id: i32 = raw
                .split('#')
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            executor.pending_summons.push(PendingSummon {
                caster_uid,
                monster_id,
            });
        }
        BehaviourType::_40006MonsterChange | BehaviourType::_40008MonsterChangeClearSelfCard => {
            let new_monster_id: i32 = raw
                .split('#')
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            for &t in &targets {
                executor.pending_monster_changes.push(PendingMonsterChange {
                    target_uid: t,
                    new_monster_id,
                });
            }
        }
        BehaviourType::_60015Kill
        | BehaviourType::_60018Kill
        | BehaviourType::_60019KillTargets
        | BehaviourType::_20012HealCantCrit
        | BehaviourType::_20016HealCantCrit
        | BehaviourType::_20018HealCantCrit
        | BehaviourType::_100017IgnoreSkillConfigDamageRate => {}
        _ => {}
    }
    vec![]
}
