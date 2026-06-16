use sonettobuf::Fight;
use crate::state::battle::{
    deck::DeckManager,
    manager::entity_mgr::EntityMgr,
};
use super::utils::make_card;

pub(crate) fn accumulate_enemy_ex_cards(
    deck_mgr: &mut DeckManager,
    fight: &Fight,
    entity_mgr: &EntityMgr,
) {
    let Some(defender) = fight.defender.as_ref() else { return };
    for e in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
        let uid = e.uid.unwrap_or(0);
        if uid >= 0 { continue; }
        let ex_skill = e.ex_skill.unwrap_or(0);
        if ex_skill == 0 { continue; }
        let required = entity_mgr.get_ex_point_required(uid);
        if required > 0 && entity_mgr.get_ex_point(uid) >= required
            && !deck_mgr.enemy_ex_deck.iter().any(|c| c.uid == Some(uid) && c.skill_id == Some(ex_skill))
            && !deck_mgr.enemy_hand.iter().any(|c| c.uid == Some(uid) && c.skill_id == Some(ex_skill))
        {
            deck_mgr.enemy_ex_deck.push(make_card(e.model_id.unwrap_or(0), ex_skill, uid, false));
        }
    }
}
