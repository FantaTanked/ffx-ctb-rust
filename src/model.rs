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
    Shield,
    Boost,
    AutoLife,
    Curse,
    Defend,
    Guard,
    Sentinel,
    Doom,
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
            "shield" => Ok(Self::Shield),
            "boost" => Ok(Self::Boost),
            "autolife" => Ok(Self::AutoLife),
            "curse" => Ok(Self::Curse),
            "defend" => Ok(Self::Defend),
            "guard" => Ok(Self::Guard),
            "sentinel" => Ok(Self::Sentinel),
            "doom" => Ok(Self::Doom),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncounterCondition {
    Preemptive,
    Normal,
    Ambush,
}
