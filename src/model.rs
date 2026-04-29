use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Character {
    Tidus,
    Yuna,
    Auron,
    Kimahri,
    Wakka,
    Lulu,
    Rikku,
    Seymour,
    Valefor,
    Ifrit,
    Ixion,
    Shiva,
    Bahamut,
    Anima,
    Yojimbo,
    Cindy,
    Sandy,
    Mindy,
    Unknown,
}

impl Character {
    pub fn input_name(self) -> &'static str {
        match self {
            Self::Tidus => "tidus",
            Self::Yuna => "yuna",
            Self::Auron => "auron",
            Self::Kimahri => "kimahri",
            Self::Wakka => "wakka",
            Self::Lulu => "lulu",
            Self::Rikku => "rikku",
            Self::Seymour => "seymour",
            Self::Valefor => "valefor",
            Self::Ifrit => "ifrit",
            Self::Ixion => "ixion",
            Self::Shiva => "shiva",
            Self::Bahamut => "bahamut",
            Self::Anima => "anima",
            Self::Yojimbo => "yojimbo",
            Self::Cindy => "cindy",
            Self::Sandy => "sandy",
            Self::Mindy => "mindy",
            Self::Unknown => "unknown",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Tidus => "Tidus",
            Self::Yuna => "Yuna",
            Self::Auron => "Auron",
            Self::Kimahri => "Kimahri",
            Self::Wakka => "Wakka",
            Self::Lulu => "Lulu",
            Self::Rikku => "Rikku",
            Self::Seymour => "Seymour",
            Self::Valefor => "Valefor",
            Self::Ifrit => "Ifrit",
            Self::Ixion => "Ixion",
            Self::Shiva => "Shiva",
            Self::Bahamut => "Bahamut",
            Self::Anima => "Anima",
            Self::Yojimbo => "Yojimbo",
            Self::Cindy => "Cindy",
            Self::Sandy => "Sandy",
            Self::Mindy => "Mindy",
            Self::Unknown => "Unknown",
        }
    }

    pub fn from_party_initial(initial: char) -> Option<Self> {
        match initial.to_ascii_lowercase() {
            't' => Some(Self::Tidus),
            'y' => Some(Self::Yuna),
            'a' => Some(Self::Auron),
            'k' => Some(Self::Kimahri),
            'w' => Some(Self::Wakka),
            'l' => Some(Self::Lulu),
            'r' => Some(Self::Rikku),
            's' => Some(Self::Seymour),
            _ => None,
        }
    }
}

impl fmt::Display for Character {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.input_name())
    }
}

impl FromStr for Character {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "tidus" => Ok(Self::Tidus),
            "yuna" => Ok(Self::Yuna),
            "auron" => Ok(Self::Auron),
            "kimahri" => Ok(Self::Kimahri),
            "wakka" => Ok(Self::Wakka),
            "lulu" => Ok(Self::Lulu),
            "rikku" => Ok(Self::Rikku),
            "seymour" => Ok(Self::Seymour),
            "valefor" => Ok(Self::Valefor),
            "ifrit" => Ok(Self::Ifrit),
            "ixion" => Ok(Self::Ixion),
            "shiva" => Ok(Self::Shiva),
            "bahamut" => Ok(Self::Bahamut),
            "anima" => Ok(Self::Anima),
            "yojimbo" => Ok(Self::Yojimbo),
            "cindy" => Ok(Self::Cindy),
            "sandy" => Ok(Self::Sandy),
            "mindy" => Ok(Self::Mindy),
            "unknown" => Ok(Self::Unknown),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonsterSlot(pub usize);

impl FromStr for MonsterSlot {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(number) = value.strip_prefix('m') else {
            return Err(());
        };
        let slot = number.parse::<usize>().map_err(|_| ())?;
        if (1..=8).contains(&slot) {
            Ok(Self(slot))
        } else {
            Err(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Death,
    Zombie,
    Eject,
    Petrify,
    Poison,
    PowerBreak,
    MagicBreak,
    ArmorBreak,
    MentalBreak,
    Confuse,
    Berserk,
    Provoke,
    Threaten,
    Sleep,
    Silence,
    Dark,
    Shell,
    Protect,
    Reflect,
    NulTide,
    NulBlaze,
    NulShock,
    NulFrost,
    Haste,
    Slow,
    Regen,
    Scan,
    PowerDistiller,
    ManaDistiller,
    SpeedDistiller,
    AbilityDistiller,
    Shield,
    Boost,
    AutoLife,
    Curse,
    Defend,
    Guard,
    Sentinel,
    Doom,
    MaxHpX2,
    MaxMpX2,
    Mp0,
    Damage9999,
    Critical,
    OverdriveX15,
    OverdriveX2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Buff {
    Cheer,
    Aim,
    Focus,
    Reflex,
    Luck,
    Jinx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Element {
    Fire,
    Ice,
    Thunder,
    Water,
    Holy,
}

impl Element {
    pub const VALUES: [Self; 5] = [
        Self::Fire,
        Self::Ice,
        Self::Thunder,
        Self::Water,
        Self::Holy,
    ];

    pub fn python_name(self) -> &'static str {
        match self {
            Self::Fire => "fire",
            Self::Ice => "ice",
            Self::Thunder => "thunder",
            Self::Water => "water",
            Self::Holy => "holy",
        }
    }
}

impl FromStr for Element {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_name(value).as_str() {
            "fire" => Ok(Self::Fire),
            "ice" => Ok(Self::Ice),
            "thunder" | "lightning" => Ok(Self::Thunder),
            "water" => Ok(Self::Water),
            "holy" => Ok(Self::Holy),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementalAffinity {
    Absorbs,
    Immune,
    Resists,
    Weak,
    Neutral,
}

impl ElementalAffinity {
    pub const VALUES: [Self; 5] = [
        Self::Absorbs,
        Self::Immune,
        Self::Resists,
        Self::Weak,
        Self::Neutral,
    ];

    pub fn python_name(self) -> &'static str {
        match self {
            Self::Absorbs => "absorbs",
            Self::Immune => "immune",
            Self::Resists => "resists",
            Self::Weak => "weak",
            Self::Neutral => "neutral",
        }
    }

    pub fn modifier_value(self) -> i32 {
        match self {
            Self::Absorbs => -1,
            Self::Immune => 0,
            Self::Resists => 1,
            Self::Neutral => 2,
            Self::Weak => 3,
        }
    }
}

impl FromStr for ElementalAffinity {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_name(value).as_str() {
            "absorbs" | "absorb" => Ok(Self::Absorbs),
            "immune" | "immunity" | "proof" => Ok(Self::Immune),
            "resists" | "resist" | "ward" => Ok(Self::Resists),
            "weak" | "weakness" => Ok(Self::Weak),
            "neutral" => Ok(Self::Neutral),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutoAbility {
    Sensor,
    FirstStrike,
    Initiative,
    Piercing,
    MagicBooster,
    Alchemy,
    HalfMpCost,
    OneMpCost,
    Hp5,
    Hp10,
    Hp20,
    Hp30,
    Mp5,
    Mp10,
    Mp20,
    Mp30,
    BreakHpLimit,
    BreakMpLimit,
    BreakDamageLimit,
    Strength3,
    Strength5,
    Strength10,
    Strength20,
    Magic3,
    Magic5,
    Magic10,
    Magic20,
    Defense3,
    Defense5,
    Defense10,
    Defense20,
    MagicDefense3,
    MagicDefense5,
    MagicDefense10,
    MagicDefense20,
    Firestrike,
    Icestrike,
    Lightningstrike,
    Waterstrike,
    FireWard,
    IceWard,
    LightningWard,
    WaterWard,
    Fireproof,
    Iceproof,
    Lightningproof,
    Waterproof,
    FireEater,
    IceEater,
    LightningEater,
    WaterEater,
    Deathproof,
    DeathWard,
    Zombieproof,
    ZombieWard,
    Stoneproof,
    StoneWard,
    Poisonproof,
    PoisonWard,
    Sleepproof,
    SleepWard,
    Silenceproof,
    SilenceWard,
    Darkproof,
    DarkWard,
    Slowproof,
    SlowWard,
    Confuseproof,
    ConfuseWard,
    Berserkproof,
    BerserkWard,
    Curseproof,
    CurseWard,
    Ribbon,
    AeonRibbon,
    Slowtouch,
    Deathtouch,
    Zombietouch,
    Stonetouch,
    Poisontouch,
    Sleeptouch,
    Silencetouch,
    Darktouch,
    Slowstrike,
    Deathstrike,
    Zombiestrike,
    Stonestrike,
    Poisonstrike,
    Sleepstrike,
    Silencestrike,
    Darkstrike,
    AutoShell,
    AutoProtect,
    AutoHaste,
    AutoRegen,
    AutoReflect,
    Counterattack,
    EvadeCounter,
    MagicCounter,
    AutoPotion,
    AutoMed,
    AutoPhoenix,
    SosShell,
    SosProtect,
    SosHaste,
    SosRegen,
    SosReflect,
    SosNulTide,
    SosNulFrost,
    SosNulShock,
    SosNulBlaze,
}

impl FromStr for AutoAbility {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_name(value).as_str() {
            "sensor" => Ok(Self::Sensor),
            "first_strike" => Ok(Self::FirstStrike),
            "initiative" => Ok(Self::Initiative),
            "piercing" => Ok(Self::Piercing),
            "magic_booster" => Ok(Self::MagicBooster),
            "alchemy" => Ok(Self::Alchemy),
            "half_mp_cost" => Ok(Self::HalfMpCost),
            "one_mp_cost" => Ok(Self::OneMpCost),
            "hp_5" => Ok(Self::Hp5),
            "hp_10" => Ok(Self::Hp10),
            "hp_20" => Ok(Self::Hp20),
            "hp_30" => Ok(Self::Hp30),
            "mp_5" => Ok(Self::Mp5),
            "mp_10" => Ok(Self::Mp10),
            "mp_20" => Ok(Self::Mp20),
            "mp_30" => Ok(Self::Mp30),
            "break_hp_limit" => Ok(Self::BreakHpLimit),
            "break_mp_limit" => Ok(Self::BreakMpLimit),
            "break_damage_limit" => Ok(Self::BreakDamageLimit),
            "strength_3" => Ok(Self::Strength3),
            "strength_5" => Ok(Self::Strength5),
            "strength_10" => Ok(Self::Strength10),
            "strength_20" => Ok(Self::Strength20),
            "magic_3" => Ok(Self::Magic3),
            "magic_5" => Ok(Self::Magic5),
            "magic_10" => Ok(Self::Magic10),
            "magic_20" => Ok(Self::Magic20),
            "defense_3" => Ok(Self::Defense3),
            "defense_5" => Ok(Self::Defense5),
            "defense_10" => Ok(Self::Defense10),
            "defense_20" => Ok(Self::Defense20),
            "magic_def_3" | "magic_defense_3" => Ok(Self::MagicDefense3),
            "magic_def_5" | "magic_defense_5" => Ok(Self::MagicDefense5),
            "magic_def_10" | "magic_defense_10" => Ok(Self::MagicDefense10),
            "magic_def_20" | "magic_defense_20" => Ok(Self::MagicDefense20),
            "firestrike" => Ok(Self::Firestrike),
            "icestrike" => Ok(Self::Icestrike),
            "lightningstrike" => Ok(Self::Lightningstrike),
            "waterstrike" => Ok(Self::Waterstrike),
            "fire_ward" | "fireward" => Ok(Self::FireWard),
            "ice_ward" | "iceward" => Ok(Self::IceWard),
            "lightning_ward" | "lightningward" => Ok(Self::LightningWard),
            "water_ward" | "waterward" => Ok(Self::WaterWard),
            "fireproof" => Ok(Self::Fireproof),
            "iceproof" => Ok(Self::Iceproof),
            "lightningproof" => Ok(Self::Lightningproof),
            "waterproof" => Ok(Self::Waterproof),
            "fire_eater" | "fireeater" => Ok(Self::FireEater),
            "ice_eater" | "iceeater" => Ok(Self::IceEater),
            "lightning_eater" | "lightningeater" => Ok(Self::LightningEater),
            "water_eater" | "watereater" => Ok(Self::WaterEater),
            "deathproof" => Ok(Self::Deathproof),
            "death_ward" | "deathward" => Ok(Self::DeathWard),
            "zombieproof" => Ok(Self::Zombieproof),
            "zombie_ward" | "zombieward" => Ok(Self::ZombieWard),
            "stoneproof" => Ok(Self::Stoneproof),
            "stone_ward" | "stoneward" => Ok(Self::StoneWard),
            "poisonproof" => Ok(Self::Poisonproof),
            "poison_ward" | "poisonward" => Ok(Self::PoisonWard),
            "sleepproof" => Ok(Self::Sleepproof),
            "sleep_ward" | "sleepward" => Ok(Self::SleepWard),
            "silenceproof" => Ok(Self::Silenceproof),
            "silence_ward" | "silenceward" => Ok(Self::SilenceWard),
            "darkproof" => Ok(Self::Darkproof),
            "dark_ward" | "darkward" => Ok(Self::DarkWard),
            "slowproof" => Ok(Self::Slowproof),
            "slow_ward" | "slowward" => Ok(Self::SlowWard),
            "confuseproof" => Ok(Self::Confuseproof),
            "confuse_ward" | "confuseward" => Ok(Self::ConfuseWard),
            "berserkproof" => Ok(Self::Berserkproof),
            "berserk_ward" | "berserkward" => Ok(Self::BerserkWard),
            "curseproof" => Ok(Self::Curseproof),
            "curse_ward" | "curseward" => Ok(Self::CurseWard),
            "ribbon" => Ok(Self::Ribbon),
            "aeon_ribbon" => Ok(Self::AeonRibbon),
            "slowtouch" => Ok(Self::Slowtouch),
            "deathtouch" => Ok(Self::Deathtouch),
            "zombietouch" => Ok(Self::Zombietouch),
            "stonetouch" => Ok(Self::Stonetouch),
            "poisontouch" => Ok(Self::Poisontouch),
            "sleeptouch" => Ok(Self::Sleeptouch),
            "silencetouch" => Ok(Self::Silencetouch),
            "darktouch" => Ok(Self::Darktouch),
            "slowstrike" => Ok(Self::Slowstrike),
            "deathstrike" => Ok(Self::Deathstrike),
            "zombiestrike" => Ok(Self::Zombiestrike),
            "stonestrike" => Ok(Self::Stonestrike),
            "poisonstrike" => Ok(Self::Poisonstrike),
            "sleepstrike" => Ok(Self::Sleepstrike),
            "silencestrike" => Ok(Self::Silencestrike),
            "darkstrike" => Ok(Self::Darkstrike),
            "auto_shell" => Ok(Self::AutoShell),
            "auto_protect" => Ok(Self::AutoProtect),
            "auto_haste" => Ok(Self::AutoHaste),
            "auto_regen" => Ok(Self::AutoRegen),
            "auto_reflect" => Ok(Self::AutoReflect),
            "counterattack" => Ok(Self::Counterattack),
            "evade_&_counter" | "evade_counter" => Ok(Self::EvadeCounter),
            "magic_counter" => Ok(Self::MagicCounter),
            "auto_potion" => Ok(Self::AutoPotion),
            "auto_med" => Ok(Self::AutoMed),
            "auto_phoenix" => Ok(Self::AutoPhoenix),
            "sos_shell" => Ok(Self::SosShell),
            "sos_protect" => Ok(Self::SosProtect),
            "sos_haste" => Ok(Self::SosHaste),
            "sos_regen" => Ok(Self::SosRegen),
            "sos_reflect" => Ok(Self::SosReflect),
            "sos_nultide" => Ok(Self::SosNulTide),
            "sos_nulfrost" => Ok(Self::SosNulFrost),
            "sos_nulshock" => Ok(Self::SosNulShock),
            "sos_nulblaze" => Ok(Self::SosNulBlaze),
            _ => Err(()),
        }
    }
}

impl FromStr for Buff {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "cheer" => Ok(Self::Cheer),
            "aim" => Ok(Self::Aim),
            "focus" => Ok(Self::Focus),
            "reflex" => Ok(Self::Reflex),
            "luck" => Ok(Self::Luck),
            "jinx" => Ok(Self::Jinx),
            _ => Err(()),
        }
    }
}

impl FromStr for Status {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "death" => Ok(Self::Death),
            "zombie" => Ok(Self::Zombie),
            "eject" => Ok(Self::Eject),
            "petrify" => Ok(Self::Petrify),
            "poison" => Ok(Self::Poison),
            "power_break" => Ok(Self::PowerBreak),
            "magic_break" => Ok(Self::MagicBreak),
            "armor_break" => Ok(Self::ArmorBreak),
            "mental_break" => Ok(Self::MentalBreak),
            "confuse" => Ok(Self::Confuse),
            "berserk" => Ok(Self::Berserk),
            "provoke" => Ok(Self::Provoke),
            "threaten" => Ok(Self::Threaten),
            "sleep" => Ok(Self::Sleep),
            "silence" => Ok(Self::Silence),
            "dark" => Ok(Self::Dark),
            "shell" => Ok(Self::Shell),
            "protect" => Ok(Self::Protect),
            "reflect" => Ok(Self::Reflect),
            "nultide" => Ok(Self::NulTide),
            "nulblaze" => Ok(Self::NulBlaze),
            "nulshock" => Ok(Self::NulShock),
            "nulfrost" => Ok(Self::NulFrost),
            "haste" => Ok(Self::Haste),
            "slow" => Ok(Self::Slow),
            "regen" => Ok(Self::Regen),
            "scan" => Ok(Self::Scan),
            "power_distiller" => Ok(Self::PowerDistiller),
            "mana_distiller" => Ok(Self::ManaDistiller),
            "speed_distiller" => Ok(Self::SpeedDistiller),
            "ability_distiller" => Ok(Self::AbilityDistiller),
            "shield" => Ok(Self::Shield),
            "boost" => Ok(Self::Boost),
            "autolife" | "auto_life" => Ok(Self::AutoLife),
            "curse" => Ok(Self::Curse),
            "defend" => Ok(Self::Defend),
            "guard" => Ok(Self::Guard),
            "sentinel" => Ok(Self::Sentinel),
            "doom" => Ok(Self::Doom),
            "max_hp_x_2" => Ok(Self::MaxHpX2),
            "max_mp_x_2" => Ok(Self::MaxMpX2),
            "mp_0" | "mp_=_0" => Ok(Self::Mp0),
            "damage_9999" => Ok(Self::Damage9999),
            "critical" => Ok(Self::Critical),
            "overdrive_x1.5" | "overdrive_x_1_5" => Ok(Self::OverdriveX15),
            "overdrive_x_2" => Ok(Self::OverdriveX2),
            _ => Err(()),
        }
    }
}

fn normalize_enum_name(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace([' ', '-', '%'], "_")
        .replace('+', "")
        .trim_matches('_')
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncounterCondition {
    Preemptive,
    Normal,
    Ambush,
}
