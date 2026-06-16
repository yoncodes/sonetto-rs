use crate::state::battle::{
    cloth::{active_cloth_level, first_melody, parse_cloth_recover_delta},
    deck::DeckManager,
    event::Event,
};
use super::traits::Manager;
use rand::rngs::StdRng;
use sonettobuf::{Fight, FightStep};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ClothMgr {
    use_counts: HashMap<i32, usize>,
}

impl ClothMgr {
    pub(crate) fn apply_power(&self, fight: &mut Fight, delta: i32) {
        let Some(cloth) = active_cloth_level(fight) else { return };
        let Some(attacker) = fight.attacker.as_mut() else { return };
        let current = attacker.power.unwrap_or(cloth.initial.max(0));
        let next = (current + delta).clamp(0, cloth.max_power.max(0));
        tracing::info!("[cloth] power {} -> {} (delta={})", current, next, delta);
        attacker.power = Some(next);
    }

    pub fn reset(&mut self) {
        self.use_counts.clear();
    }

    pub fn execute_skill(
        &mut self,
        skill_id: i32,
        fight: &mut Fight,
        deck_mgr: &mut DeckManager,
        rng: &mut StdRng,
    ) -> anyhow::Result<Vec<FightStep>> {
        let cloth = active_cloth_level(fight).ok_or_else(|| anyhow::anyhow!("no cloth"))?;

        let cost_vec = if skill_id == cloth.skill1 {
            cloth.use_power1.clone()
        } else if skill_id == cloth.skill2 {
            cloth.use_power2.clone()
        } else {
            anyhow::bail!("cloth skill {skill_id} unimplemented")
        };

        let count = self.use_counts.entry(skill_id).or_insert(0);
        let cost = cost_vec.get(*count).copied().unwrap_or_else(|| cost_vec.last().copied().unwrap_or(0));
        *count += 1;

        self.apply_power(fight, -(cost as i32));

        let steps = match skill_id {
            30010201 => first_melody::universal_card(skill_id, deck_mgr),
            30010202 => first_melody::redeal_card(skill_id, fight, deck_mgr, rng),
            _ => anyhow::bail!("cloth skill {skill_id} unimplemented"),
        };

        Ok(steps)
    }
}

impl Manager for ClothMgr {
    fn on_battle_start(&mut self, fight: &mut Fight) {
        let Some(cloth) = active_cloth_level(fight) else { return };
        if let Some(attacker) = fight.attacker.as_mut() {
            if attacker.power.is_none() {
                let initial = cloth.initial.max(0);
                tracing::info!("[cloth] battle_start power={}", initial);
                attacker.power = Some(initial);
            }
        }
    }

    fn on_round_end(&mut self, fight: &mut Fight) {
        let Some(cloth) = active_cloth_level(fight) else { return };
        let round_index = fight.cur_round.unwrap_or(1);
        let delta = parse_cloth_recover_delta(&cloth.recover, round_index);
        tracing::info!("[cloth] round_end round={} recover_delta={}", round_index, delta);
        if delta != 0 {
            self.apply_power(fight, delta);
        }
    }

    fn on_use_card(&mut self, fight: &Fight, mut events: Vec<Event>) -> Vec<Event> {
        let delta = active_cloth_level(fight).map_or(0, |c| c.r#use.max(0));
        if delta != 0 { events.push(Event::PowerChange { delta }); }
        events
    }

    fn on_move_card(&mut self, fight: &Fight, mut events: Vec<Event>) -> Vec<Event> {
        let delta = active_cloth_level(fight).map_or(0, |c| c.r#move.max(0));
        if delta != 0 { events.push(Event::PowerChange { delta }); }
        events
    }

    fn on_card_upgrade(&mut self, fight: &Fight, mut events: Vec<Event>) -> Vec<Event> {
        let delta = active_cloth_level(fight).map_or(0, |c| c.compose.max(0));
        if delta != 0 { events.push(Event::PowerChange { delta }); }
        events
    }
}
