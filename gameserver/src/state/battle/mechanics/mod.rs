pub mod advanced_cure;
pub mod bloodtithe;
pub mod channel;
pub mod dot;
pub mod dot_settle_round_start;
pub mod empathy;
pub mod injury_counter;
pub mod magic_circle;
pub mod nautika;
pub mod phase_change;
pub mod shadowcloak;

use bloodtithe::BloodtitheState;
use channel::ChannelState;
use empathy::EmpathyState;
use phase_change::PhaseChangeState;
use shadowcloak::ShadowCloakState;

use crate::state::battle::emission_timeline::EmissionTimeline;
use crate::state::battle::manager::{buff_mgr::BuffMgr, entity_mgr::EntityMgr};
use sonettobuf::{Fight, FightStep};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mechanics {
    pub bloodtithe: BloodtitheState,
    pub channel: ChannelState,
    pub empathy: EmpathyState,
    pub phase_change: PhaseChangeState,
    pub shadow_cloak: ShadowCloakState,
    /// Round-scoped emission timeline. Read-only debug accounting:
    /// every skill emission appends one record. Cleared at round
    /// open; dumped at round end when `SONETTO_EMISSION_TIMELINE=1`.
    /// Behavior is unaffected by recording.
    pub emission_timeline: EmissionTimeline,
}

impl Mechanics {
    pub fn new() -> Self {
        Self {
            bloodtithe: BloodtitheState::new(),
            channel: ChannelState::new(),
            empathy: EmpathyState::new(),
            phase_change: PhaseChangeState::new(),
            shadow_cloak: ShadowCloakState::new(),
            emission_timeline: EmissionTimeline::new(),
        }
    }

    pub fn init(&mut self, fight: &Fight) {
        self.channel.init(fight);
        self.empathy.init(fight);
        self.shadow_cloak.init(fight);
    }

    /// Pull live buff-act state from `BuffMgr` into mechanics caches.
    ///
    /// Some `BuffMgr::set_instance_act_common_params` writes (e.g.
    /// Kakania's Empathy storage updates) only land on the runtime
    /// BuffMgr — the per-entity `BuffInfo` snapshot in `fight.entity.
    /// buffs` is not currently kept in sync. Without this refresh,
    /// re-initializing mechanics from `Fight` between rounds resets
    /// the stored values to the pre-round snapshot. Run this AFTER
    /// `init(&Fight)` so the cumulative totals match what the engine
    /// actually wrote during the round being simulated.
    pub fn sync_from_buff_mgr(&mut self, buff_mgr: &BuffMgr) {
        self.empathy.sync_from_buff_mgr(buff_mgr);
    }

    pub fn on_bloodpool_init(&self) -> Option<FightStep> {
        self.bloodtithe.bloodpool_init_step()
    }

    pub fn on_pre_raspberry(&self) -> Option<FightStep> {
        self.bloodtithe.bloodtithe_sync_step()
    }

    pub fn on_raspberry(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        entity_mgr: &mut EntityMgr,
    ) -> Option<FightStep> {
        self.bloodtithe
            .raspberry_step(fight, buff_mgr, entity_mgr, &mut self.shadow_cloak)
    }

    pub fn on_post_raspberry(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        entity_mgr: &EntityMgr,
    ) -> Option<FightStep> {
        self.shadow_cloak.sync_step(fight, buff_mgr, entity_mgr)
    }
}
