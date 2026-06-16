use crate::state::battle::heroes::pickles;
use config::configs;
use sonettobuf::Fight;
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct CollectedPassives {
    pub attacker: Vec<(i64, Vec<i32>)>,
    pub defender: Vec<(i64, Vec<i32>)>,
    pub battle_attacker: Vec<i32>,
    pub battle_defender: Vec<i32>,
}

impl CollectedPassives {
    pub fn is_empty(&self) -> bool {
        self.attacker.is_empty()
            && self.defender.is_empty()
            && self.battle_attacker.is_empty()
            && self.battle_defender.is_empty()
    }

    pub fn merged_for(&self, uid: i64) -> Vec<i32> {
        self.attacker
            .iter()
            .chain(self.defender.iter())
            .find(|(u, _)| *u == uid)
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default()
    }

    pub fn attacker_uids(&self) -> Vec<i64> {
        self.attacker.iter().map(|(uid, _)| *uid).collect()
    }

    pub fn defender_uids(&self) -> Vec<i64> {
        self.defender.iter().map(|(uid, _)| *uid).collect()
    }

    pub fn filter<F: Fn(i32) -> bool>(&self, keep: F) -> Self {
        let filter_side = |side: &Vec<(i64, Vec<i32>)>| -> Vec<(i64, Vec<i32>)> {
            side.iter()
                .map(|(uid, ids)| (*uid, ids.iter().copied().filter(|sid| keep(*sid)).collect()))
                .filter(|(_, ids): &(i64, Vec<i32>)| !ids.is_empty())
                .collect()
        };
        Self {
            attacker: filter_side(&self.attacker),
            defender: filter_side(&self.defender),
            battle_attacker: self
                .battle_attacker
                .iter()
                .copied()
                .filter(|sid| keep(*sid))
                .collect(),
            battle_defender: self
                .battle_defender
                .iter()
                .copied()
                .filter(|sid| keep(*sid))
                .collect(),
        }
    }
}

pub fn collect(_fight: &Fight, _battle_id: i32) -> CollectedPassives {
    // Battle rules are managed by RuleMgr; passive collection is no longer needed here.
    CollectedPassives::default()
}

fn collect_side_passives(
    fight: &Fight,
    is_attacker: bool,
    exclude_skill_ids: Option<&HashSet<i32>>,
) -> Vec<(i64, Vec<i32>)> {
    let side = if is_attacker {
        fight.attacker.as_ref()
    } else {
        fight.defender.as_ref()
    };
    side.map(|a| {
        a.entitys
            .iter()
            .chain(a.sub_entitys.iter())
            .filter_map(|e| {
                tracing::info!(
                    "collect_side_passives is_attacker={} uid={:?} position={:?} passive_skill={:?}",
                    is_attacker, e.uid, e.position, e.passive_skill
                );
                let uid = e.uid?;
                if e.passive_skill.is_empty() {
                    return None;
                }
                if e.position.unwrap_or(-1) <= 0 {
                    return None;
                }
                let mut passive_skill = e.passive_skill.clone();
                if pickles::is_pickles(e.model_id)
                    && passive_skill.contains(&30630161)
                    && !passive_skill.contains(&30630171)
                {
                    passive_skill.push(30630171);
                }
                if let Some(exclude) = exclude_skill_ids {
                    passive_skill.retain(|sid| !exclude.contains(sid));
                }
                if passive_skill.is_empty() {
                    return None;
                }
                Some((uid, passive_skill))
            })
            .collect()
    })
    .unwrap_or_default()
}

pub struct BattlePassives {
    pub attacker: Vec<i32>,
    pub defender: Vec<i32>,
}

pub fn collect_battle_passives(battle_id: i32) -> BattlePassives {
    let game = configs::get();
    let Some(battle) = game.battle.iter().find(|b| b.id == battle_id) else {
        return BattlePassives {
            attacker: vec![],
            defender: vec![],
        };
    };
    if battle.addition_rule.is_empty() {
        return BattlePassives {
            attacker: vec![],
            defender: vec![],
        };
    }
    let mut attacker = Vec::new();
    let mut defender = Vec::new();
    for entry in battle.addition_rule.split('|') {
        let mut parts = entry.split('#');
        let Some(prefix) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
            continue;
        };
        let Some(id) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
            continue;
        };
        let rule = match prefix {
            1..=3 => game.rule.iter().find(|r| r.id == id),
            _ => {
                tracing::warn!(
                    "collect_battle_passives: unknown prefix {} in battle {}",
                    prefix,
                    battle_id
                );
                None
            }
        };

        // Route by additionRule prefix:
        // attacker gets all entries; defender gets only `3#...` entries.
        // If a rule row is missing, fall back to id directly.
        if let Some(rule) = rule {
            let sid = rule.effect.parse::<i32>().ok().unwrap_or(0);
            if sid == 0 {
                continue;
            }
            attacker.push(sid);
            if prefix == 3 {
                defender.push(sid);
            }
        } else if id != 0 {
            attacker.push(id);
            if prefix == 3 {
                defender.push(id);
            }
        }
    }
    BattlePassives { attacker, defender }
}

pub fn inject_battle_passives(fight: &mut Fight, passives: &BattlePassives) {
    if let Some(attacker) = fight.attacker.as_mut() {
        for entity in attacker
            .entitys
            .iter_mut()
            .chain(attacker.sub_entitys.iter_mut())
        {
            // attackers — append at end
            for &skill_id in &passives.attacker {
                if !entity.passive_skill.contains(&skill_id) {
                    entity.passive_skill.push(skill_id);
                }
            }
        }
    }
    if let Some(defender) = fight.defender.as_mut() {
        for entity in defender
            .entitys
            .iter_mut()
            .chain(defender.sub_entitys.iter_mut())
        {
            // defenders — prepend at start
            let mut insert_pos = 0;
            for &skill_id in &passives.defender {
                if !entity.passive_skill.contains(&skill_id) {
                    entity.passive_skill.insert(insert_pos, skill_id);
                    insert_pos += 1;
                }
            }
        }
    }
}
