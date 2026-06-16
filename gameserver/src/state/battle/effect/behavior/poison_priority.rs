//! AddTargetBuffByPoison — prioritize poison applications onto the
//! enemy side's current poison carriers instead of blindly fanning out.
//!
//! Willow's poison passives use behavior `60112`. LIVE applies these
//! stacks one at a time, re-checking which enemy currently carries the
//! most poison after each application. When targets tie, the action
//! prefers the current hostile focus (`behavior_ctx.target_uid`), which
//! matches battle3 r2 where both extra Willow stacks stay on `-1`.

use sonettobuf::{ActEffect, Fight};

use crate::state::battle::event::Event;
use crate::state::battle::manager::fight_data_mgr::Managers;
use crate::state::battle::mechanics::Mechanics;
use crate::state::battle::skill::SkillExecutor;
use crate::state::battle::buff::apply_skill::{self as buff, BuffApplySpec};
use crate::state::battle::skill::targets::{alive_enemies_by_position, get_entity};
use rand::rngs::StdRng;

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    entity_uid: i64,
    skill_id: i32,
    raw: &str,
) -> Vec<Event> {
    let mut parts = raw.split('#').skip(1);
    let buff_id: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let stack_count: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let _duration: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let max_targets: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1);

    let total_stacks = stack_count.max(0) as usize;
    if total_stacks == 0 || buff_id == 0 {
        return vec![];
    }

    let has_bloodpool = mechanics.bloodtithe.has_bloodpool();
    let preferred_uid = entity_uid;
    let mut pool = rank_enemies(
        fight,
        managers,
        alive_enemies_by_position(fight, entity_uid),
        preferred_uid,
    );
    pool.truncate(max_targets.max(1) as usize);

    if pool.is_empty() {
        return vec![];
    }

    let has_existing_poison = pool
        .iter()
        .any(|&uid| poison_instance_count(managers, uid) > 0);
    let mut effects: Vec<ActEffect> = Vec::new();

    if has_existing_poison {
        for _ in 0..total_stacks {
            let target_uid = rank_enemies(fight, managers, pool.clone(), preferred_uid)
                .into_iter()
                .next()
                .unwrap_or(pool[0]);
            effects.extend(buff::apply(
                BuffApplySpec::new(buff_id)
                    .caster(entity_uid)
                    .target(target_uid)
                    .count(1)
                    .bloodpool(has_bloodpool)
                    .skill(skill_id),
                executor,
                fight,
                managers,
                mechanics,
            ));
        }
    } else {
        for idx in 0..total_stacks {
            let target_uid = pool[idx % pool.len()];
            effects.extend(buff::apply(
                BuffApplySpec::new(buff_id)
                    .caster(entity_uid)
                    .target(target_uid)
                    .count(1)
                    .bloodpool(has_bloodpool)
                    .skill(skill_id),
                executor,
                fight,
                managers,
                mechanics,
            ));
        }
    }

    effects
        .into_iter()
        .map(|e| Event::SerializedActEffect { effect: e })
        .collect()
}

fn rank_enemies(
    fight: &Fight,
    managers: &Managers,
    mut uids: Vec<i64>,
    preferred_uid: i64,
) -> Vec<i64> {
    uids.sort_by(|&a, &b| {
        let poison_a = poison_instance_count(managers, a);
        let poison_b = poison_instance_count(managers, b);
        poison_b
            .cmp(&poison_a)
            .then_with(|| (b == preferred_uid).cmp(&(a == preferred_uid)))
            .then_with(|| enemy_position(fight, a).cmp(&enemy_position(fight, b)))
            .then_with(|| a.cmp(&b))
    });
    uids
}

fn enemy_position(fight: &Fight, uid: i64) -> i32 {
    get_entity(fight, uid)
        .and_then(|e| e.position)
        .unwrap_or(99)
}

fn poison_instance_count(managers: &Managers, uid: i64) -> i32 {
    managers
        .buff_mgr
        .get(uid)
        .iter()
        .filter(|i| is_poison_family(i.buff_id))
        .map(|i| i.layer.max(1))
        .sum()
}

fn is_poison_family(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    buff_cfg.features.split('|').any(|entry| {
        entry
            .split('#')
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
            .is_some_and(|act_id| matches!(act_id, 803 | 844))
    })
}
