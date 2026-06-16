use crate::state::battle::event::Event;
use sonettobuf::Fight;

pub trait Manager {
    fn on_battle_start(&mut self, _fight: &mut Fight) {}
    fn on_round_start(&mut self) {}
    fn on_round_end(&mut self, _fight: &mut Fight) {}
    fn on_battle_end(&mut self) {}
    fn on_enter_fight(&mut self, _fight: &Fight, _entity_uid: i64) -> Vec<Event> { vec![] }
    fn on_use_card(&mut self, _fight: &Fight, events: Vec<Event>) -> Vec<Event> { events }
    fn on_move_card(&mut self, _fight: &Fight, events: Vec<Event>) -> Vec<Event> { events }
    fn on_card_upgrade(&mut self, _fight: &Fight, events: Vec<Event>) -> Vec<Event> { events }
    fn on_dead(&mut self, _fight: &Fight, _entity_uid: i64) -> Vec<Event> { vec![] }
}
