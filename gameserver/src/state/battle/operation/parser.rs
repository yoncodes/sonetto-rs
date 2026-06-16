use sonettobuf::{BeginRoundOper, CardInfo, Fight};
use crate::state::battle::{
    card::{CardOpType, apply_card_upgrades, upgrade_level1},
    deck::DeckManager,
    event::Event,
};

pub struct ParsedOps {
    pub selected_cards: Vec<CardInfo>,
    pub player_events: Vec<Event>,
}

pub fn parse_round_open_ops(
    operations: &[BeginRoundOper],
    deck_mgr: &mut DeckManager,
    fight: &Fight,
) -> ParsedOps {
    let mut selected_pairs: Vec<(usize, CardInfo)> = Vec::new();
    let mut player_events: Vec<Event> = Vec::new();

    for op in operations {
        match CardOpType::try_from(op.oper_type.unwrap_or(0)) {
            Ok(CardOpType::MoveCard) if op.to_id.unwrap_or(0) == 0 => {
                let from = (op.param1.unwrap_or(1) - 1) as usize;
                let to   = (op.param2.unwrap_or(1) - 1) as usize;
                tracing::warn!("  move idx={} -> idx={} (deck size {})", from, to, deck_mgr.player_hand.len());
                if from < deck_mgr.player_hand.len() && to < deck_mgr.player_hand.len() {
                    let card = deck_mgr.player_hand[from].clone();
                    player_events.push(Event::CardMoved { card: card.clone() });
                    let moved = deck_mgr.player_hand.remove(from);
                    deck_mgr.player_hand.insert(to, moved);
                    let upgrades = apply_card_upgrades(&mut deck_mgr.player_hand, fight);
                    for _ in 0..upgrades {
                        player_events.push(Event::CardUpgrade { card: card.clone() });
                    }
                }
            }
            Ok(CardOpType::PlayCard)
            | Ok(CardOpType::MoveCard)
            | Ok(CardOpType::AssistBoss)
            | Ok(CardOpType::PlayerFinisherSkill)
            | Ok(CardOpType::BloodPool) => {
                let idx = (op.param1.unwrap_or(1) - 1) as usize;
                tracing::warn!("  pick idx={} from deck of {} cards:", idx, deck_mgr.player_hand.len());
                for (i, c) in deck_mgr.player_hand.iter().enumerate() {
                    tracing::warn!("    [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
                }
                if idx < deck_mgr.player_hand.len() {
                    let card = deck_mgr.player_hand.remove(idx);
                    tracing::warn!("  -> selected uid={:?} skill={:?}", card.uid, card.skill_id);
                    player_events.push(Event::CardPlayed { card: card.clone(), oper: op.clone() });
                    selected_pairs.push((selected_pairs.len(), card.clone()));
                    let upgrades = apply_card_upgrades(&mut deck_mgr.player_hand, fight);
                    for _ in 0..upgrades {
                        player_events.push(Event::CardUpgrade { card: card.clone() });
                    }
                } else {
                    tracing::warn!("  -> idx {} OUT OF RANGE (deck size {})", idx, deck_mgr.player_hand.len());
                }
            }
            Ok(CardOpType::MoveUniversal) => {
                let universal_idx = (op.param1.unwrap_or(1) - 1) as usize;
                let target_idx    = (op.param2.unwrap_or(1) - 1) as usize;
                let is_universal  = deck_mgr.player_hand.get(universal_idx)
                    .and_then(|c| c.skill_id)
                    .map_or(false, |id| id == 30000001);
                if is_universal && target_idx < deck_mgr.player_hand.len() {
                    if let Some(next_skill) = upgrade_level1(&deck_mgr.player_hand[target_idx], fight) {
                        deck_mgr.player_hand[target_idx].skill_id = Some(next_skill);
                        deck_mgr.player_hand.remove(universal_idx);
                    }
                }
                tracing::warn!("  upgrade idx={} with universal idx={} (is_universal={})", target_idx, universal_idx, is_universal);
            }
            Ok(CardOpType::SimulateDissolveCard) => {
                player_events.push(Event::SimulateDissolveCard { oper: op.clone() });
            }
            _ => {}
        }
    }

    ParsedOps {
        selected_cards: selected_pairs.into_iter().map(|(_, c)| c).collect(),
        player_events,
    }
}
