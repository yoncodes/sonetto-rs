//! CatapultBuff action — apply a buff to the primary target, then fan
//! out additional stacks to other random legal enemies.
//!
//! Tuesday's `The Horror's Delight` uses this behavior for Poison
//! spread. The runtime keeps the normal buff-application path so the
//! standard BuffAdd / BuffUpdate and marker emissions (e.g. Poison 213)
//! still come from `buff::apply`.

use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    event::Event,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{SkillExecutor, buff, targets::alive_enemies},
    types::condition::ConditionType,
};
use rand::{rngs::StdRng, seq::SliceRandom};
use sonettobuf::Fight;

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    targets: Vec<i64>,
    entity_uid: i64,
    raw: &str,
    _count: i32,
    _beh_type: BehaviourType,
) -> Vec<Event> {
    let parts: Vec<&str> = raw.split('#').collect();
    let primary_stacks: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let buff_id: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
    let catapult_stacks: i32 = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);
    let catapult_cap: i32 = parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);

    let has_bloodpool = mechanics.bloodtithe.has_bloodpool();
    let target = targets.into_iter().next().unwrap_or(0);
    let mut out = Vec::new();

    // LIVE emits one BuffAdd per stack (count=0 layer=0), not a
    // single BuffAdd with layer=N. Calling `buff::apply` once with
    // `count=N` would collapse N stacks into a single emission with
    // `layer=N`, which is the wrong shape — match LIVE by calling
    // `buff::apply` per stack with count=1.
    for _ in 0..primary_stacks.max(0) {
        out.extend(
            buff::apply(
                buff::BuffApplySpec::new(buff_id)
                    .caster(entity_uid)
                    .target(target)
                    .count(1)
                    .bloodpool(has_bloodpool)
                    .skill(0)
                    .condition(0, &ConditionType::None),
                executor,
                fight,
                managers,
                mechanics,
            )
            .into_iter()
            .map(|e| Event::SerializedActEffect { effect: e }),
        );
    }

    // The catapult spread can land on ANY alive enemy including the
    // primary target. Verified from battle3 r2 step[8] LIVE shape:
    // primary -1 gets 2 initial stacks, then catapult lands on -2
    // and bounces back to -1, producing 4 BuffAdds total (2 +
    // catapult_cap=2 spreads).
    let mut all_enemies = alive_enemies(fight, entity_uid);
    all_enemies.shuffle(rng);

    for enemy_uid in all_enemies.into_iter().take(catapult_cap.max(0) as usize) {
        for _ in 0..catapult_stacks.max(0) {
            out.extend(
                buff::apply(
                    buff::BuffApplySpec::new(buff_id)
                        .caster(entity_uid)
                        .target(enemy_uid)
                        .count(1)
                        .bloodpool(has_bloodpool)
                        .skill(0)
                        .condition(0, &ConditionType::None),
                    executor,
                    fight,
                    managers,
                    mechanics,
                )
                .into_iter()
                .map(|e| Event::SerializedActEffect { effect: e }),
            );
        }
    }

    out
}
