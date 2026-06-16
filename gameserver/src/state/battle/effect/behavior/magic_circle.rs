//! MagicCircle action — handler for the two behavior variants that
//! interact with the magic-circle / array-skill system:
//!
//! * `AddMagicCircle { circle_id }` — summon a magic circle. Emits a
//!   `MagicCircleAdd` ActEffect carrying the circle config plus,
//!   optionally, a `BuffAdd` for the circle's `self_buff` (and a
//!   per-ally `CureUpByLostHp` pair when the self_buff features a
//!   CureUpByLostHp act). Delegates to
//!   `mechanics::magic_circle::add_magic_circle`.
//! * `MagicCircleAttr { modifiers }` — array-skill attr aura. The
//!   variant's `modifiers` field carries `(side, attr_id, permille)`
//!   tuples parsed from `60076#side#attr#permille[#side2#attr2#permille2]`.
//!   The handler is **currently no-op pending fixture data**. Empirical
//!   testing against battle1 r1 with a per-target `Attr(26)` emission
//!   produced six extra effects vs LIVE — meaning LIVE does NOT emit
//!   per-target Attr(26) markers for this behavior. The correct
//!   emission shape is unknown until a fixture with active magic-circle
//!   /array-skill psychubes is captured. For now the handler preserves
//!   the parsed `modifiers` data structure so the future implementation
//!   can pick it up without re-deriving from raw parts.

use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::{
    buff_actions::EffectContext, event::Event, manager::fight_data_mgr::Managers,
    mechanics::Mechanics, mechanics::magic_circle as magic_circle_mechanic, skill::SkillExecutor,
};
use rand::rngs::StdRng;
use sonettobuf::Fight;

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    targets: Vec<i64>,
    entity_uid: i64,
    raw: &str,
    _count: i32,
    beh_type: BehaviourType,
) -> Vec<Event> {
    match beh_type {
        BehaviourType::_50019AddMagicCircle | BehaviourType::_60163MagicCircleAddRound => {
            let circle_id: i32 = raw
                .split('#')
                .nth(1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let target = targets.into_iter().next().unwrap_or(0);
            let mut ctx = EffectContext::new(fight, managers, mechanics, entity_uid, target);
            magic_circle_mechanic::add_magic_circle(
                &mut ctx, executor, fight, entity_uid, circle_id,
            )
            .unwrap_or_default()
            .into_iter()
            .map(|e| Event::SerializedActEffect { effect: e })
            .collect()
        }
        // TODO: implement when a fixture with active magic-circle
        // psychubes is available to validate against. The
        // `modifiers` field is parsed but currently unused.
        BehaviourType::_60076MagicCircleAttr
        | BehaviourType::_50020RemoveAllMagicCircle
        | BehaviourType::_50021RemoveMagicCircleById
        | BehaviourType::_60270UpdateWangQiMagicCircle
        | BehaviourType::_60272ChangeElectricMagicCircleProgress => vec![],
        _ => vec![],
    }
}
