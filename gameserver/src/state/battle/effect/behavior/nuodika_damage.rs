//! NuoDiKaDamage action — Nautika's Dual Faith damage variant.
//!
//! The variant carries `(primary_buff_id, primary_rate,
//! secondary_buff_id, secondary_rate, self_loss_param)`. The buff
//! ids are looked up in `buff_actions::attr_replace` to produce
//! per-buff permille values; the two are combined with their rates
//! into a `total_permille` of caster max HP. The caster takes
//! `current_hp × (self_loss_param/5) %` self-damage, then targets
//! resolved through `TargetResolver` each take
//! `max_hp × total_permille / 1000`.
use crate::state::battle::effect::behavior::r#type::BehaviourType;
use crate::state::battle::skill::cache::resolve_skill_effect_id;
use crate::state::battle::{
    buff_actions::attr_replace::buff_get_attr_replace_permille,
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    mechanics::Mechanics,
    skill::{
        SkillExecutor,
        targets::{TargetResolver, get_entity},
    },
};
use rand::rngs::StdRng;
use sonettobuf::Fight;

pub fn execute(
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor,
    _rng: &mut StdRng,
    targets: Vec<i64>,
    entity_uid: i64,
    skill_id: i32,
    raw: &str,
    _count: i32,
    _beh_type: BehaviourType,
) -> Vec<Event> {
    let _ = managers;
    let _ = mechanics;
    let parts: Vec<&str> = raw.split('#').collect();
    let primary_buff_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let primary_rate: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    let secondary_buff_id: i32 = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
    let secondary_rate: i32 = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
    let self_loss_param: i32 = parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0);

    let target = targets.into_iter().next().unwrap_or(0);

    let Some(caster) = get_entity(fight, entity_uid) else {
        return vec![];
    };
    let current_hp = caster.current_hp.unwrap_or(0);
    let max_hp = caster
        .attr
        .as_ref()
        .and_then(|a| a.hp)
        .unwrap_or(current_hp)
        .max(current_hp)
        .max(0);

    let primary_permille = buff_get_attr_replace_permille(primary_buff_id).unwrap_or(0);
    let secondary_permille = buff_get_attr_replace_permille(secondary_buff_id).unwrap_or(0);
    let total_permille = (primary_permille.saturating_mul(primary_rate) / 1000)
        .saturating_add(secondary_permille.saturating_mul(secondary_rate) / 1000)
        .max(0);
    let self_loss = current_hp.saturating_mul((self_loss_param / 5).max(0)) / 100;

    let cfg = config::configs::get();
    let _ = cfg;
    let logic_target = cfg
        .skill_effect
        .iter()
        .find(|s| s.id == resolve_skill_effect_id(skill_id))
        .and_then(|s| s.logic_target.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let damage_targets = TargetResolver::new(fight, entity_uid, target)
        .behavior(logic_target)
        .resolve();

    let mut out = Vec::new();
    if self_loss > 0 {
        out.push(Event::SerializedActEffect {
            effect: ActEffectBuilder::damage_skill(entity_uid, self_loss, 30006, 0, entity_uid),
        });
    }
    if total_permille <= 0 {
        return out;
    }
    let damage = (max_hp.saturating_mul(total_permille) / 1000).max(1);
    for damage_target in damage_targets {
        if damage_target == 0 || damage_target == entity_uid {
            continue;
        }
        out.push(Event::SerializedActEffect {
            effect: ActEffectBuilder::damage_skill(damage_target, damage, -1, 0, entity_uid),
        });
    }
    out
}
