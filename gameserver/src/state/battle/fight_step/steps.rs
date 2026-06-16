use sonettobuf::{ActEffect, FightStep, fight_step};

pub const STEP_EFFECT_COUNT_LIMIT: usize = 50;

pub fn make_skill_step(
    caster_uid: i64,
    target_uid: i64,
    skill_id: i32,
    card_index: i32,
    effects: Vec<ActEffect>,
) -> FightStep {
    FightStep {
        act_type: Some(fight_step::ActType::Skill.into()),
        from_id: Some(caster_uid),
        to_id: Some(target_uid),
        act_id: Some(skill_id),
        act_effect: effects,
        card_index: Some(card_index + 1),
        support_hero_id: Some(0),
        fake_timeline: Some(false),
        real_skill_type: Some(0),
        real_skin_id: Some(0),
    }
}

pub fn split_step_by_effect_limit(step: FightStep) -> Vec<FightStep> {
    if step.act_effect.len() <= STEP_EFFECT_COUNT_LIMIT {
        return vec![step];
    }

    let mut out = Vec::new();
    for chunk in step.act_effect.chunks(STEP_EFFECT_COUNT_LIMIT) {
        let mut s = step.clone();
        s.act_effect = chunk.to_vec();
        out.push(s);
    }
    out
}
