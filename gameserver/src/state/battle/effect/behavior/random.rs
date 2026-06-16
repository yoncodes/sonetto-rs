use anyhow::Result;
use rand::{rngs::StdRng, seq::SliceRandom};
use sonettobuf::{ActEffect, Fight};
use std::collections::HashSet;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{SkillExecutor, buff},
};
use crate::state::battle::effect::behavior::r#type::BehaviourType;

/// Pick `count` buffs from the meta-buff pool keyed by `pool_buff_id`,
/// biased toward buffs the target does not already have. The bias is the
/// generic mechanic behind multi-round random-buff rotations (e.g. Pickles'
/// Tier I Euphoria "Collection of Buffs"): re-firing the same passive across
/// rounds should cycle through the pool rather than repeatedly picking the
/// same prefix. Within each partition (missing vs. already-present) the
/// selection is randomized via the simulator's seeded `StdRng`, so output
/// remains deterministic across runs.
#[allow(clippy::too_many_arguments)]
pub fn add_buff_ran_id(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    caster_uid: i64,
    target: i64,
    pool_buff_id: i32,
    count: i32,
) -> Result<Vec<ActEffect>> {
    let cfg = config::configs::get();
    let pool: Vec<i32> = cfg
        .skill_buff
        .iter()
        .find(|b| b.id == pool_buff_id)
        .map(|b| {
            b.features
                .split('#')
                .filter_map(|entry| entry.split(',').next()?.parse().ok())
                .collect()
        })
        .unwrap_or_default();

    if pool.is_empty() {
        return Ok(vec![]);
    }

    // Snapshot the buffs currently on the target so we can prefer pool
    // entries the target doesn't already have. Comparison is by buff_id
    // — if the target already carries multiple instances of the same
    // pool entry, all of them collapse into a single "already_have"
    // marker, which matches LIVE's intent ("favor what's missing").
    let target_buff_ids: HashSet<i32> = managers
        .buff_mgr
        .get(target)
        .iter()
        .map(|buff| buff.buff_id)
        .collect();

    let (mut missing, mut already_have): (Vec<i32>, Vec<i32>) = pool
        .into_iter()
        .partition(|buff_id| !target_buff_ids.contains(buff_id));

    missing.shuffle(rng);
    already_have.shuffle(rng);

    let count = count.max(0) as usize;
    let mut chosen = Vec::with_capacity(count);
    chosen.extend(missing.into_iter().take(count));
    if chosen.len() < count {
        chosen.extend(already_have.into_iter().take(count - chosen.len()));
    }

    let has_bloodpool = mechanics.bloodtithe.has_bloodpool();
    let mut effects = Vec::new();
    for buff_id in chosen {
        effects.extend(buff::apply(
            buff::BuffApplySpec::new(buff_id)
                .caster(caster_uid)
                .target(target)
                .bloodpool(has_bloodpool),
            executor,
            fight,
            managers,
            mechanics,
        ));
    }
    Ok(effects)
}

pub fn execute(
    fight: &Fight, managers: &mut Managers, mechanics: &mut Mechanics,
    executor: &mut SkillExecutor, rng: &mut StdRng,
    targets: Vec<i64>, entity_uid: i64, raw: &str, beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_20021AddBuffRanId => {
            let parts: Vec<&str> = raw.split('#').collect();
            let pool_buff_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            let count: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            targets.into_iter().flat_map(|target| {
                add_buff_ran_id(executor, rng, fight, managers, mechanics, entity_uid, target, pool_buff_id, count)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| Event::SerializedActEffect { effect: e })
            }).collect()
        }
        BehaviourType::_20022AddBuffRanTypeId | BehaviourType::_20023AddBuffRanTypeGroup => {
            tracing::warn!("unimplemented behaviour type: {:?}", beh_type);
            vec![]
        }
        _ => vec![],
    }
}
