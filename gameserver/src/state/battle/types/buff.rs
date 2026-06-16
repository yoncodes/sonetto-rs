/// Buff layer/halo type — stored in buff.type field.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BuffLayerType {
    Normal = 0,
    LayerMasterHalo = 1,
    LayerSlaveHalo = 2,
}

#[allow(dead_code)]
impl BuffLayerType {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Normal),
            1 => Some(Self::LayerMasterHalo),
            2 => Some(Self::LayerSlaveHalo),
            _ => None,
        }
    }
}

/// Buff alignment — used by Disperse and Purify.
/// Source: FightEnum.FightBuffType
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BuffAlignment {
    Bad = 1,
    Good = 2,
    Normal = 3,
}

#[allow(dead_code)]
impl BuffAlignment {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            1 => Some(Self::Bad),
            2 => Some(Self::Good),
            3 => Some(Self::Normal),
            _ => None,
        }
    }
}

/// `includeTypes` — stored in `skill_bufftype.includeTypes`.
/// Format: `<base>` or `<base>#<N>` where N is the max stack count.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum IncludeType {
    /// One instance; re-applying replaces it. e.g. "Restless Heart" (crit buff).
    Unique = 0,
    /// Multiple sources coexist as separate instances. e.g. "Fanged Partner".
    Normal = 1,
    /// Consume-on-trigger; each proc removes 1 stack. e.g. "Rousing Morale" (DMG+50%).
    ConsumeOnProc = 2,
    /// Counter / reactive; triggers when the carrier is attacked. e.g. "Corroding".
    Counter = 3,
    /// Simple stat; one instance per stat slot. e.g. "ATK Up".
    Stat = 4,
    /// Magic-circle / field passive; attached to the field, not an entity. e.g. Tuesday array.
    FieldPassive = 5,
    /// Shared slot; only one of this bufftype at a time. e.g. "DMG Bonus Up", "Shield".
    SharedSlot = 6,
    /// Hidden passive (not shown in UI). e.g. "Explore Buff".
    HiddenPassive = 7,
    /// Conditional debuff; effect depends on attack type. e.g. "Blind" (single-target only).
    Conditional = 8,
    /// Stackable; each application pushes a new instance, N = max stacks.
    /// e.g. "Aura of Exuberance I" (stacks each round, up to N).
    Stacked = 10,
    /// Stacked (ritual variant). e.g. "Ritual Oracle: Detachment".
    StackedRitual = 11,
    /// Stacked, consume-on-trigger. e.g. "Ill-Omen: Expose" (consumed on proc).
    StackedConsume = 12,
    /// Stacked, undispellable. e.g. "Saturn" (Penetration Rate +6%, cannot be dispelled).
    StackedFixed = 13,
    /// Stacked, -1 per hit, N = max. e.g. "Poison Feather" (max 2 stacks, -1 per hit).
    StackedDecay = 14,
    /// Force-field stacked. e.g. "Cosmic Energy" (force field, up to N layers).
    StackedField = 15,
}

#[allow(dead_code)]
impl IncludeType {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Unique),
            1 => Some(Self::Normal),
            2 => Some(Self::ConsumeOnProc),
            3 => Some(Self::Counter),
            4 => Some(Self::Stat),
            5 => Some(Self::FieldPassive),
            6 => Some(Self::SharedSlot),
            7 => Some(Self::HiddenPassive),
            8 => Some(Self::Conditional),
            10 => Some(Self::Stacked),
            11 => Some(Self::StackedRitual),
            12 => Some(Self::StackedConsume),
            13 => Some(Self::StackedFixed),
            14 => Some(Self::StackedDecay),
            15 => Some(Self::StackedField),
            _ => None,
        }
    }

    pub fn is_stackable(self) -> bool {
        matches!(
            self,
            Self::Stacked
                | Self::StackedRitual
                | Self::StackedConsume
                | Self::StackedFixed
                | Self::StackedDecay
                | Self::StackedField
        )
    }
}

pub mod stack_type {
    pub fn is_stackable(include_types: &str) -> bool {
        include_types
            // includeTypes can be encoded as plain ids ("10")
            // or id-with-arg forms ("10#3"). Treat either as stackable ids.
            .split([',', '，', '#'])
            .any(|v| matches!(v.trim(), "10" | "12" | "14" | "15"))
    }
}

/// One clause from `skill_bufftype.excludeTypes`.
/// Format: `1#<typeId,...>` or `2#<buffId,...>`, multiple clauses separated by `|`.
/// When a buff with this type is applied, all matching buffs on the target are removed first.
///
/// Examples:
/// - `"1#7"`            → remove all Shield (type=7) buffs  ("Shield" replaces existing shield)
/// - `"1#1,5"`          → remove all StatsUp + PosStatus    ("Awake" strips positive statuses)
/// - `"1#4"`            → remove all Control buffs          ("Hyper" self-cleanses control)
/// - `"2#5012"`         → remove specific buff id 5012      ("Empower II" replaces "Empower I")
/// - `"2#2130005|1#7"`  → remove buff 2130005 AND all type=7 ("Vajra's Boon" replaces variants)
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcludeRule {
    /// `1#<type_id,...>` — remove all buffs whose `bufftype.type` is in the list.
    ByCategory(Vec<i32>),
    /// `2#<buff_id,...>` — remove buffs with specific `buff_id`s.
    ByBuffId(Vec<i32>),
}

#[allow(dead_code)]
impl ExcludeRule {
    /// Parse `skill_bufftype.excludeTypes` into a list of rules.
    /// Returns empty vec if the field is empty.
    pub fn parse(s: &str) -> Vec<Self> {
        if s.is_empty() { return vec![]; }
        s.split('|').filter_map(|clause| {
            let mut parts = clause.splitn(2, '#');
            let kind: i32 = parts.next()?.trim().parse().ok()?;
            let ids: Vec<i32> = parts.next().unwrap_or("").split(',')
                .filter_map(|v| v.trim().parse().ok()).collect();
            if ids.is_empty() { return None; }
            match kind {
                1 => Some(Self::ByCategory(ids)),
                2 => Some(Self::ByBuffId(ids)),
                _ => None,
            }
        }).collect()
    }
}

/// `takeStage` — stored in `skill_bufftype.takeStage`.
/// The event at which this buff is applied / takes effect.
/// `-1` = always-active passive (stat modifier, no trigger event).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TakeStage {
    /// Always-active passive — stat modifier with no trigger. e.g. "ATK +5%", "DEF +10%".
    AlwaysActive = -1,
    /// Applied immediately / on round start. e.g. "Corrupt: Frenzy".
    Immediate    = 0,
    /// Round start. e.g. "Hold Breath".
    RoundStart   = 102,
    /// When a buff is gained (self-applied). e.g. "ATK Up", "ATK Down".
    OnBuffGain   = 103,
    /// On heal received. e.g. "Nebular Collapse".
    OnHeal       = 104,
    /// On skill use decision. e.g. "Level Up Decision".
    OnSkillUse   = 105,
    /// On action / turn start. e.g. "Preparation - Spathodea I".
    OnAction     = 106,
    /// On round-start heal tick. e.g. "Sotheby Round Extension".
    RoundStartHeal = 210,
    /// On death. e.g. "Carbuncle Reincarnation".
    OnDeath      = 301,
    /// On being attacked (debuff application). e.g. "ATK Down", "Shield".
    OnAttacked   = 303,
    /// On attacking. e.g. "Attack Decision Tag".
    OnAttack     = 304,
    /// Invincibility window. e.g. "Invincibility".
    Invincible   = 307,
}

#[allow(dead_code)]
impl TakeStage {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            -1 => Some(Self::AlwaysActive),
            0 => Some(Self::Immediate),
            102 => Some(Self::RoundStart),
            103 => Some(Self::OnBuffGain),
            104 => Some(Self::OnHeal),
            105 => Some(Self::OnSkillUse),
            106 => Some(Self::OnAction),
            210 => Some(Self::RoundStartHeal),
            301 => Some(Self::OnDeath),
            303 => Some(Self::OnAttacked),
            304 => Some(Self::OnAttack),
            307 => Some(Self::Invincible),
            _ => None,
        }
    }
}
/// The trigger that consumes one stack of this buff (decrements count).
/// Format: `<base>` or `<base>#<skillType,...>` to restrict to specific skill types.
///
/// Skill type filter values (after `#`): 1=Basic, 2=Signature, 3=Ultimate, 5=Resonance.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TakeActBase {
    /// Consumed on any attack. e.g. "Rousing Morale" (DMG+50%).
    OnAttack       = 0,
    /// Consumed after attacking. e.g. "Silver Bullet" (DEF Down on hit).
    AfterAttack    = 1,
    /// Consumed when being attacked. e.g. "Party Balloon" (reflect DMG).
    OnBeingAttacked = 2,
    /// Consumed on lethal DMG (survival trigger). e.g. "Prayer".
    OnLethalDmg    = 6,
    /// Consumed on taking Reality DMG. e.g. "Cold Resistance".
    OnRealityDmg   = 7,
    /// Consumed on taking Mental DMG. e.g. "Mental Boon".
    OnMentalDmg    = 8,
    /// Consumed on using an incantation (optionally filtered by skill type). e.g. "Thief Master".
    OnUseIncantation = 9,
    /// Consumed on being hit by an incantation (optionally filtered by skill type). e.g. "Ill-Omen: Expose".
    OnHitByIncantation = 10,
    /// Consumed on round end. e.g. "Burn", "Poison".
    OnRoundEnd     = 13,
    /// Consumed on scoring a critical hit. e.g. "Discernment".
    OnCrit         = 14,
    /// Consumed on dodging. e.g. "Instant Concealment".
    OnDodge        = 15,
    /// Consumed on being hit by an Ultimate (optionally filtered). e.g. "Plappy".
    OnHitByUltimate = 12,
}

#[allow(dead_code)]
impl TakeActBase {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::OnAttack),
            1 => Some(Self::AfterAttack),
            2 => Some(Self::OnBeingAttacked),
            6 => Some(Self::OnLethalDmg),
            7 => Some(Self::OnRealityDmg),
            8 => Some(Self::OnMentalDmg),
            9 => Some(Self::OnUseIncantation),
            10 => Some(Self::OnHitByIncantation),
            12 => Some(Self::OnHitByUltimate),
            13 => Some(Self::OnRoundEnd),
            14 => Some(Self::OnCrit),
            15 => Some(Self::OnDodge),
            _ => None,
        }
    }
}

/// Source: FightEnum.BuffTypeList
pub const GOOD_BUFF_TYPES: &[i32] = &[1, 3, 5];
pub const BAD_BUFF_TYPES: &[i32] = &[2, 4, 6];

/// Buff category — maps `skill_bufftype.type` field.
/// English names from `skill_buff_desc` + `language_en`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BuffCategory {
    /// 属性提升 — "Stats Up". e.g. "ATK Up", "DEF Up".
    StatsUp = 1,
    /// 属性削弱 — "Stats Down". e.g. "ATK Down".
    StatsDown = 2,
    /// 反制 — "Counter". e.g. "Taunt", "Corroding".
    Counter = 3,
    /// 控制 — "Control". e.g. "Stun", "Freeze", "Sleep".
    Control = 4,
    /// 状态增益 — "Pos Status". e.g. "Rousing Morale", "Shield".
    PosStatus = 5,
    /// 状态异常 — "Neg Status". e.g. "Poison", "Burn", "Bleed".
    NegStatus = 6,
    /// 护盾 — "Shield". e.g. "Shield" (absorbs damage).
    Shield = 7,
    /// 特殊 — "Special". e.g. immunity, unique mechanics.
    Special = 8,
    /// 被动 — "Passive". Passive effects, shown in UI.
    Passive = 9,
    /// 力场 — "Force Field". e.g. "Cosmic Energy" force-field layers.
    ForceField = 10,
    /// 被动（不显示） — Passive, hidden from UI.
    PassiveHidden = 11,
    /// 被动（不显示） — Passive, hidden from UI (second variant).
    PassiveHidden2 = 12,
    /// 心相（不显示） — Psychube buff, hidden from UI.
    Psychube = 13,
    /// 吟诵 — "Channel". Chanting/incantation buffs.
    Channel = 14,
}

#[allow(dead_code)]
impl BuffCategory {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            1 => Some(Self::StatsUp),
            2 => Some(Self::StatsDown),
            3 => Some(Self::Counter),
            4 => Some(Self::Control),
            5 => Some(Self::PosStatus),
            6 => Some(Self::NegStatus),
            7 => Some(Self::Shield),
            8 => Some(Self::Special),
            9 => Some(Self::Passive),
            10 => Some(Self::ForceField),
            11 => Some(Self::PassiveHidden),
            12 => Some(Self::PassiveHidden2),
            13 => Some(Self::Psychube),
            14 => Some(Self::Channel),
            _ => None,
        }
    }
}
