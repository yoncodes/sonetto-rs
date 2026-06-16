use std::collections::HashSet;
use sonettobuf::CardInfo;

pub(crate) fn purge_dead_entity_cards(deck: &mut Vec<CardInfo>, alive_uids: &HashSet<i64>) {
    deck.retain(|c| {
        let uid = c.uid.unwrap_or(0);
        uid == 0 || c.temp_card.unwrap_or(false) || alive_uids.contains(&uid)
    });
}

pub(crate) fn purge_and_maybe_rebuild_deck(
    deck: &mut Vec<CardInfo>,
    alive_uids: &HashSet<i64>,
    rebuild: impl FnOnce() -> Vec<CardInfo>,
) {
    if deck
        .iter()
        .any(|c| c.uid.map_or(false, |u| !alive_uids.contains(&u)))
    {
        *deck = rebuild();
    }
}
