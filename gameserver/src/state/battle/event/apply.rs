use sonettobuf::FightStep;
use crate::state::battle::{
    context::FightContext,
    event::Event,
    fight_step::{ActEffectBuilder, FightStepBuilder},
};

pub fn events_to_steps(events: Vec<Event>) -> Vec<FightStep> {
    use crate::state::battle::event::events_to_act_effects;
    events_to_act_effects(events)
        .into_iter()
        .map(|effect| FightStepBuilder::effect().with(effect).build())
        .collect()
}
