//! Which enemies can appear at a random enemy marker.
//!
//! When the game builds a dungeon it replaces every random enemy marker with
//! an enemy chosen from a table for the dungeon's type, using a dice roll and
//! the player's level. The tables are built into the original executable
//! (`FALL.EXE`), not the data files. They and the selection rules are taken from
//! Daggerfall Unity (MIT licence), which reproduces them from the original game.

/// A dungeon type, as stored for each location in `MAPS.BSA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DungeonType(u8);

/// Number of dungeon types.
const DUNGEON_TYPE_COUNT: usize = 19;
/// Number of entries in every dungeon encounter table, ordered from weakest to strongest.
const TABLE_LENGTH: usize = 20;

/// Names of the dungeon types, indexed by type number.
const DUNGEON_TYPE_NAMES: [&str; DUNGEON_TYPE_COUNT] = [
    "Crypt",
    "Orc Stronghold",
    "Human Stronghold",
    "Prison",
    "Desecrated Temple",
    "Mine",
    "Natural Cave",
    "Coven",
    "Vampire Haunt",
    "Laboratory",
    "Harpy Nest",
    "Ruined Castle",
    "Spider Nest",
    "Giant Stronghold",
    "Dragon's Den",
    "Barbarian Stronghold",
    "Volcanic Caves",
    "Scorpion Nest",
    "Cemetery",
];

impl DungeonType {
    /// The dungeon type stored as `value`, or `None` for a location without a dungeon (255) or an unknown value.
    pub fn from_raw(value: u8) -> Option<Self> {
        (usize::from(value) < DUNGEON_TYPE_COUNT).then_some(Self(value))
    }

    /// Every dungeon type, in type number order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..DUNGEON_TYPE_COUNT as u8).map(Self)
    }

    /// The type number stored in the game data.
    pub fn raw(self) -> u8 {
        self.0
    }

    /// Human readable name, for example "Orc Stronghold".
    pub fn name(self) -> &'static str {
        DUNGEON_TYPE_NAMES[usize::from(self.0)]
    }

    /// The dungeon type called `name` (ignoring case), if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::all().find(|dungeon_type| dungeon_type.name().eq_ignore_ascii_case(name))
    }

    /// The enemies that can appear at random enemy markers, weakest first.
    pub fn encounter_table(self) -> &'static [Enemy; TABLE_LENGTH] {
        &ENCOUNTER_TABLES[usize::from(self.0)]
    }
}

/// An enemy type ("mobile"), numbered as in the game data: monsters 0-42, human classes 128-146.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Enemy {
    Rat = 0,
    Imp = 1,
    Spriggan = 2,
    GiantBat = 3,
    GrizzlyBear = 4,
    SabertoothTiger = 5,
    Spider = 6,
    Orc = 7,
    Centaur = 8,
    Werewolf = 9,
    Nymph = 10,
    Slaughterfish = 11,
    OrcSergeant = 12,
    Harpy = 13,
    Wereboar = 14,
    SkeletalWarrior = 15,
    Giant = 16,
    Zombie = 17,
    Ghost = 18,
    Mummy = 19,
    GiantScorpion = 20,
    OrcShaman = 21,
    Gargoyle = 22,
    Wraith = 23,
    OrcWarlord = 24,
    FrostDaedra = 25,
    FireDaedra = 26,
    Daedroth = 27,
    Vampire = 28,
    DaedraSeducer = 29,
    VampireAncient = 30,
    DaedraLord = 31,
    Lich = 32,
    AncientLich = 33,
    Dragonling = 34,
    FireAtronach = 35,
    IronAtronach = 36,
    FleshAtronach = 37,
    IceAtronach = 38,
    HorseInvalid = 39,
    DragonlingAlternate = 40,
    Dreugh = 41,
    Lamia = 42,
    Mage = 128,
    Spellsword = 129,
    Battlemage = 130,
    Sorcerer = 131,
    Healer = 132,
    Nightblade = 133,
    Bard = 134,
    Burglar = 135,
    Rogue = 136,
    Acrobat = 137,
    Thief = 138,
    Assassin = 139,
    Monk = 140,
    Archer = 141,
    Ranger = 142,
    Barbarian = 143,
    Warrior = 144,
    Knight = 145,
    KnightCityWatch = 146,
}

/// Id of the first human enemy class.
const FIRST_HUMAN_ID: u16 = 128;

impl Enemy {
    /// Human readable name, for example "Skeletal Warrior".
    pub fn name(self) -> &'static str {
        match self {
            Self::Rat => "Rat",
            Self::Imp => "Imp",
            Self::Spriggan => "Spriggan",
            Self::GiantBat => "Giant Bat",
            Self::GrizzlyBear => "Grizzly Bear",
            Self::SabertoothTiger => "Sabertooth Tiger",
            Self::Spider => "Spider",
            Self::Orc => "Orc",
            Self::Centaur => "Centaur",
            Self::Werewolf => "Werewolf",
            Self::Nymph => "Nymph",
            Self::Slaughterfish => "Slaughterfish",
            Self::OrcSergeant => "Orc Sergeant",
            Self::Harpy => "Harpy",
            Self::Wereboar => "Wereboar",
            Self::SkeletalWarrior => "Skeletal Warrior",
            Self::Giant => "Giant",
            Self::Zombie => "Zombie",
            Self::Ghost => "Ghost",
            Self::Mummy => "Mummy",
            Self::GiantScorpion => "Giant Scorpion",
            Self::OrcShaman => "Orc Shaman",
            Self::Gargoyle => "Gargoyle",
            Self::Wraith => "Wraith",
            Self::OrcWarlord => "Orc Warlord",
            Self::FrostDaedra => "Frost Daedra",
            Self::FireDaedra => "Fire Daedra",
            Self::Daedroth => "Daedroth",
            Self::Vampire => "Vampire",
            Self::DaedraSeducer => "Daedra Seducer",
            Self::VampireAncient => "Vampire Ancient",
            Self::DaedraLord => "Daedra Lord",
            Self::Lich => "Lich",
            Self::AncientLich => "Ancient Lich",
            Self::Dragonling => "Dragonling",
            Self::FireAtronach => "Fire Atronach",
            Self::IronAtronach => "Iron Atronach",
            Self::FleshAtronach => "Flesh Atronach",
            Self::IceAtronach => "Ice Atronach",
            Self::HorseInvalid => "Horse (invalid)",
            Self::DragonlingAlternate => "Dragonling",
            Self::Dreugh => "Dreugh",
            Self::Lamia => "Lamia",
            Self::Mage => "Mage",
            Self::Spellsword => "Spellsword",
            Self::Battlemage => "Battlemage",
            Self::Sorcerer => "Sorcerer",
            Self::Healer => "Healer",
            Self::Nightblade => "Nightblade",
            Self::Bard => "Bard",
            Self::Burglar => "Burglar",
            Self::Rogue => "Rogue",
            Self::Acrobat => "Acrobat",
            Self::Thief => "Thief",
            Self::Assassin => "Assassin",
            Self::Monk => "Monk",
            Self::Archer => "Archer",
            Self::Ranger => "Ranger",
            Self::Barbarian => "Barbarian",
            Self::Warrior => "Warrior",
            Self::Knight => "Knight",
            Self::KnightCityWatch => "City Watch",
        }
    }

    /// Every enemy type, monsters first, in id order.
    pub fn all() -> impl Iterator<Item = Self> {
        ALL_ENEMIES.into_iter()
    }

    /// Whether the game can spawn the enemy. The unused horse has no graphics
    /// and crashes the original game when spawned.
    pub fn can_spawn(self) -> bool {
        self != Self::HorseInvalid
    }

    /// The enemy with game id `mobile_id`, if there is one.
    pub fn from_mobile_id(mobile_id: u16) -> Option<Self> {
        ALL_ENEMIES.into_iter().find(|enemy| enemy.mobile_id() == mobile_id)
    }

    /// The enemy's id in the game data.
    pub fn mobile_id(self) -> u16 {
        self as u16
    }

    /// Whether the enemy is a human class (mage, knight, …) rather than a monster.
    pub fn is_human(self) -> bool {
        self.mobile_id() >= FIRST_HUMAN_ID
    }

    /// Whether the enemy lives in water; the game only places it when its marker is under water.
    pub fn lives_in_water(self) -> bool {
        matches!(self, Self::Slaughterfish | Self::Dreugh | Self::Lamia)
    }
}

/// Every enemy type, in id order.
const ALL_ENEMIES: [Enemy; ENEMY_COUNT] = [
    Enemy::Rat,
    Enemy::Imp,
    Enemy::Spriggan,
    Enemy::GiantBat,
    Enemy::GrizzlyBear,
    Enemy::SabertoothTiger,
    Enemy::Spider,
    Enemy::Orc,
    Enemy::Centaur,
    Enemy::Werewolf,
    Enemy::Nymph,
    Enemy::Slaughterfish,
    Enemy::OrcSergeant,
    Enemy::Harpy,
    Enemy::Wereboar,
    Enemy::SkeletalWarrior,
    Enemy::Giant,
    Enemy::Zombie,
    Enemy::Ghost,
    Enemy::Mummy,
    Enemy::GiantScorpion,
    Enemy::OrcShaman,
    Enemy::Gargoyle,
    Enemy::Wraith,
    Enemy::OrcWarlord,
    Enemy::FrostDaedra,
    Enemy::FireDaedra,
    Enemy::Daedroth,
    Enemy::Vampire,
    Enemy::DaedraSeducer,
    Enemy::VampireAncient,
    Enemy::DaedraLord,
    Enemy::Lich,
    Enemy::AncientLich,
    Enemy::Dragonling,
    Enemy::FireAtronach,
    Enemy::IronAtronach,
    Enemy::FleshAtronach,
    Enemy::IceAtronach,
    Enemy::HorseInvalid,
    Enemy::DragonlingAlternate,
    Enemy::Dreugh,
    Enemy::Lamia,
    Enemy::Mage,
    Enemy::Spellsword,
    Enemy::Battlemage,
    Enemy::Sorcerer,
    Enemy::Healer,
    Enemy::Nightblade,
    Enemy::Bard,
    Enemy::Burglar,
    Enemy::Rogue,
    Enemy::Acrobat,
    Enemy::Thief,
    Enemy::Assassin,
    Enemy::Monk,
    Enemy::Archer,
    Enemy::Ranger,
    Enemy::Barbarian,
    Enemy::Warrior,
    Enemy::Knight,
    Enemy::KnightCityWatch,
];

/// Number of enemy types.
const ENEMY_COUNT: usize = 62;

/// The chance of one enemy appearing at a random enemy marker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyChance {
    pub enemy: Enemy,
    /// Between 0 and 1.
    pub chance: f32,
}

/// The encounter table a random enemy marker picks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncounterTable {
    /// The table for the dungeon's type.
    Dungeon(DungeonType),
    /// The table for markers below the cell's water level, whatever the dungeon type.
    Underwater,
}

impl EncounterTable {
    /// The table's enemies, weakest first.
    pub fn enemies(self) -> &'static [Enemy; TABLE_LENGTH] {
        match self {
            Self::Dungeon(dungeon_type) => dungeon_type.encounter_table(),
            Self::Underwater => &UNDERWATER_TABLE,
        }
    }
}

/// The chances of each enemy appearing at a random enemy marker that picks
/// from `table`, for a player of `player_level`, most likely first.
///
/// Follows the original game's dungeon pick, which Daggerfall Unity uses by
/// default: a roll of 1-100 picks a range of the table around the player's
/// level, then an entry in that range is picked evenly.
pub fn enemy_chances(table: EncounterTable, player_level: u32) -> Vec<EnemyChance> {
    let level = i64::from(player_level);
    let last = TABLE_LENGTH as i64 - 1;
    // (chance of the roll, lowest entry, highest entry) before the range is clamped.
    let rolls = [
        (0.80, level - 3, level + 3),
        (0.15, 0, level + 1),
        (0.05, 0, if level <= 5 { level + 2 } else { level + 1 }),
    ];

    let mut chances: Vec<EnemyChance> = Vec::new();
    for (roll_chance, mut minimum, mut maximum) in rolls {
        if minimum < 0 {
            (minimum, maximum) = (0, 5);
        } else if maximum > last {
            (minimum, maximum) = (last - 5, last);
        }
        let entry_chance = roll_chance / (maximum - minimum + 1) as f32;
        for index in minimum..=maximum {
            let enemy = table.enemies()[index as usize];
            match chances.iter_mut().find(|chance| chance.enemy == enemy) {
                Some(existing) => existing.chance += entry_chance,
                None => chances.push(EnemyChance { enemy, chance: entry_chance }),
            }
        }
    }
    // A stable sort keeps weaker enemies first among equal chances.
    chances.sort_by(|first, second| second.chance.total_cmp(&first.chance));
    chances
}

/// Enemies at random enemy markers below a cell's water level, weakest first.
const UNDERWATER_TABLE: [Enemy; TABLE_LENGTH] = [
    Enemy::Slaughterfish,
    Enemy::Slaughterfish,
    Enemy::SkeletalWarrior,
    Enemy::Slaughterfish,
    Enemy::Slaughterfish,
    Enemy::Dreugh,
    Enemy::Dreugh,
    Enemy::Slaughterfish,
    Enemy::Zombie,
    Enemy::Dreugh,
    Enemy::Slaughterfish,
    Enemy::IceAtronach,
    Enemy::Slaughterfish,
    Enemy::Dreugh,
    Enemy::Lamia,
    Enemy::Dreugh,
    Enemy::Lamia,
    Enemy::Zombie,
    Enemy::Wraith,
    Enemy::Ghost,
];

/// Encounter tables for each dungeon type, indexed by type number, weakest enemy first.
const ENCOUNTER_TABLES: [[Enemy; TABLE_LENGTH]; DUNGEON_TYPE_COUNT] = [
    // Crypt
    [
        Enemy::Rat,
        Enemy::SkeletalWarrior,
        Enemy::GiantBat,
        Enemy::Rat,
        Enemy::SkeletalWarrior,
        Enemy::GiantBat,
        Enemy::Mummy,
        Enemy::SkeletalWarrior,
        Enemy::Spider,
        Enemy::Zombie,
        Enemy::Ghost,
        Enemy::Zombie,
        Enemy::Zombie,
        Enemy::Ghost,
        Enemy::Ghost,
        Enemy::Wraith,
        Enemy::Wraith,
        Enemy::Vampire,
        Enemy::VampireAncient,
        Enemy::Lich,
    ],
    // Orc Stronghold
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Orc,
        Enemy::OrcSergeant,
        Enemy::GrizzlyBear,
        Enemy::Orc,
        Enemy::SkeletalWarrior,
        Enemy::Orc,
        Enemy::OrcSergeant,
        Enemy::SabertoothTiger,
        Enemy::Giant,
        Enemy::OrcShaman,
        Enemy::Spider,
        Enemy::OrcShaman,
        Enemy::OrcWarlord,
        Enemy::GiantScorpion,
        Enemy::Daedroth,
        Enemy::GiantScorpion,
        Enemy::OrcShaman,
        Enemy::OrcWarlord,
    ],
    // Human Stronghold
    [
        Enemy::Warrior,
        Enemy::Rogue,
        Enemy::Rat,
        Enemy::GrizzlyBear,
        Enemy::Archer,
        Enemy::Nightblade,
        Enemy::GiantBat,
        Enemy::Spellsword,
        Enemy::Centaur,
        Enemy::Knight,
        Enemy::Warrior,
        Enemy::Rogue,
        Enemy::Barbarian,
        Enemy::OrcWarlord,
        Enemy::Archer,
        Enemy::Wraith,
        Enemy::Spellsword,
        Enemy::OrcShaman,
        Enemy::Warrior,
        Enemy::Vampire,
    ],
    // Prison
    [
        Enemy::GiantBat,
        Enemy::Rat,
        Enemy::Bard,
        Enemy::SkeletalWarrior,
        Enemy::Burglar,
        Enemy::Spider,
        Enemy::Nightblade,
        Enemy::Spider,
        Enemy::Barbarian,
        Enemy::Thief,
        Enemy::FleshAtronach,
        Enemy::Thief,
        Enemy::Assassin,
        Enemy::Zombie,
        Enemy::Nightblade,
        Enemy::IronAtronach,
        Enemy::Wraith,
        Enemy::Ghost,
        Enemy::Zombie,
        Enemy::Burglar,
    ],
    // Desecrated Temple
    [
        Enemy::GiantBat,
        Enemy::Imp,
        Enemy::Healer,
        Enemy::Monk,
        Enemy::Sorcerer,
        Enemy::SkeletalWarrior,
        Enemy::Harpy,
        Enemy::Monk,
        Enemy::Mummy,
        Enemy::Monk,
        Enemy::OrcShaman,
        Enemy::Gargoyle,
        Enemy::Wraith,
        Enemy::Daedroth,
        Enemy::DaedraSeducer,
        Enemy::Monk,
        Enemy::FrostDaedra,
        Enemy::FireDaedra,
        Enemy::Dragonling,
        Enemy::DaedraLord,
    ],
    // Mine
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::GrizzlyBear,
        Enemy::Spider,
        Enemy::SkeletalWarrior,
        Enemy::Spider,
        Enemy::Nightblade,
        Enemy::Orc,
        Enemy::OrcSergeant,
        Enemy::Giant,
        Enemy::Thief,
        Enemy::Giant,
        Enemy::Warrior,
        Enemy::IceAtronach,
        Enemy::IronAtronach,
        Enemy::FireDaedra,
        Enemy::Thief,
        Enemy::Vampire,
        Enemy::Nightblade,
        Enemy::VampireAncient,
    ],
    // Natural Cave
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::GrizzlyBear,
        Enemy::SabertoothTiger,
        Enemy::GrizzlyBear,
        Enemy::Spider,
        Enemy::Orc,
        Enemy::Werewolf,
        Enemy::Barbarian,
        Enemy::Harpy,
        Enemy::Wereboar,
        Enemy::Giant,
        Enemy::GiantScorpion,
        Enemy::OrcShaman,
        Enemy::Ghost,
        Enemy::Warrior,
        Enemy::Monk,
        Enemy::Barbarian,
        Enemy::Dragonling,
        Enemy::Lich,
    ],
    // Coven
    [
        Enemy::Imp,
        Enemy::GiantBat,
        Enemy::Mage,
        Enemy::Sorcerer,
        Enemy::Harpy,
        Enemy::Nightblade,
        Enemy::Nymph,
        Enemy::FleshAtronach,
        Enemy::FireAtronach,
        Enemy::IronAtronach,
        Enemy::Nymph,
        Enemy::Daedroth,
        Enemy::Sorcerer,
        Enemy::Spellsword,
        Enemy::DaedraSeducer,
        Enemy::FireDaedra,
        Enemy::Battlemage,
        Enemy::Mage,
        Enemy::FrostDaedra,
        Enemy::DaedraLord,
    ],
    // Vampire Haunt
    [
        Enemy::Rat,
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Spider,
        Enemy::Werewolf,
        Enemy::Nightblade,
        Enemy::SkeletalWarrior,
        Enemy::SkeletalWarrior,
        Enemy::Ghost,
        Enemy::Mummy,
        Enemy::Wraith,
        Enemy::Ghost,
        Enemy::Vampire,
        Enemy::Wraith,
        Enemy::Vampire,
        Enemy::Vampire,
        Enemy::VampireAncient,
        Enemy::Vampire,
        Enemy::VampireAncient,
        Enemy::VampireAncient,
    ],
    // Laboratory
    [
        Enemy::Imp,
        Enemy::Mage,
        Enemy::Sorcerer,
        Enemy::Battlemage,
        Enemy::Zombie,
        Enemy::Mage,
        Enemy::Sorcerer,
        Enemy::FleshAtronach,
        Enemy::Battlemage,
        Enemy::IceAtronach,
        Enemy::FireAtronach,
        Enemy::IronAtronach,
        Enemy::Gargoyle,
        Enemy::Daedroth,
        Enemy::Sorcerer,
        Enemy::Lich,
        Enemy::Mage,
        Enemy::Sorcerer,
        Enemy::Lich,
        Enemy::AncientLich,
    ],
    // Harpy Nest
    [
        Enemy::Rat,
        Enemy::Warrior,
        Enemy::GiantBat,
        Enemy::Harpy,
        Enemy::GiantBat,
        Enemy::Harpy,
        Enemy::SkeletalWarrior,
        Enemy::Spider,
        Enemy::Burglar,
        Enemy::Harpy,
        Enemy::GiantScorpion,
        Enemy::OrcShaman,
        Enemy::Nightblade,
        Enemy::Harpy,
        Enemy::Rogue,
        Enemy::Vampire,
        Enemy::DaedraSeducer,
        Enemy::Harpy,
        Enemy::DaedraSeducer,
        Enemy::DaedraLord,
    ],
    // Ruined Castle
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Warrior,
        Enemy::Orc,
        Enemy::SkeletalWarrior,
        Enemy::Spider,
        Enemy::OrcSergeant,
        Enemy::Werewolf,
        Enemy::Knight,
        Enemy::Wereboar,
        Enemy::Zombie,
        Enemy::Giant,
        Enemy::Knight,
        Enemy::Ghost,
        Enemy::Wraith,
        Enemy::Warrior,
        Enemy::Knight,
        Enemy::Lich,
        Enemy::AncientLich,
        Enemy::VampireAncient,
    ],
    // Spider Nest
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::GiantBat,
        Enemy::Spider,
        Enemy::SkeletalWarrior,
        Enemy::Spriggan,
        Enemy::Ranger,
        Enemy::Spider,
        Enemy::GiantScorpion,
        Enemy::Rogue,
        Enemy::Harpy,
        Enemy::Spider,
        Enemy::Thief,
        Enemy::Ghost,
        Enemy::Mummy,
        Enemy::Spider,
        Enemy::Wraith,
        Enemy::Spider,
        Enemy::Assassin,
        Enemy::Lich,
    ],
    // Giant Stronghold
    [
        Enemy::Rat,
        Enemy::GrizzlyBear,
        Enemy::SabertoothTiger,
        Enemy::Orc,
        Enemy::Giant,
        Enemy::OrcSergeant,
        Enemy::GiantScorpion,
        Enemy::Giant,
        Enemy::Wereboar,
        Enemy::Giant,
        Enemy::OrcWarlord,
        Enemy::GiantScorpion,
        Enemy::OrcShaman,
        Enemy::Giant,
        Enemy::Daedroth,
        Enemy::FireDaedra,
        Enemy::Dragonling,
        Enemy::Giant,
        Enemy::OrcWarlord,
        Enemy::FrostDaedra,
    ],
    // Dragon's Den
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Centaur,
        Enemy::Burglar,
        Enemy::GiantScorpion,
        Enemy::Werewolf,
        Enemy::Knight,
        Enemy::Harpy,
        Enemy::Spider,
        Enemy::Gargoyle,
        Enemy::Sorcerer,
        Enemy::Nymph,
        Enemy::Nightblade,
        Enemy::Mage,
        Enemy::Knight,
        Enemy::Vampire,
        Enemy::Battlemage,
        Enemy::Thief,
        Enemy::AncientLich,
        Enemy::DaedraLord,
    ],
    // Barbarian Stronghold
    [
        Enemy::Warrior,
        Enemy::Barbarian,
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Warrior,
        Enemy::Centaur,
        Enemy::Rogue,
        Enemy::Archer,
        Enemy::Werewolf,
        Enemy::Barbarian,
        Enemy::Archer,
        Enemy::Wereboar,
        Enemy::Spider,
        Enemy::Warrior,
        Enemy::Vampire,
        Enemy::Rogue,
        Enemy::Barbarian,
        Enemy::VampireAncient,
        Enemy::Sorcerer,
        Enemy::AncientLich,
    ],
    // Volcanic Caves
    [
        Enemy::GiantBat,
        Enemy::Imp,
        Enemy::SabertoothTiger,
        Enemy::SabertoothTiger,
        Enemy::Barbarian,
        Enemy::FireAtronach,
        Enemy::Harpy,
        Enemy::Wereboar,
        Enemy::Giant,
        Enemy::Ghost,
        Enemy::GiantScorpion,
        Enemy::IronAtronach,
        Enemy::FireAtronach,
        Enemy::Mage,
        Enemy::IronAtronach,
        Enemy::Wraith,
        Enemy::Daedroth,
        Enemy::FireDaedra,
        Enemy::DaedraLord,
        Enemy::AncientLich,
    ],
    // Scorpion Nest
    [
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Spider,
        Enemy::SkeletalWarrior,
        Enemy::GiantScorpion,
        Enemy::Thief,
        Enemy::GiantScorpion,
        Enemy::Zombie,
        Enemy::Imp,
        Enemy::Ghost,
        Enemy::GiantScorpion,
        Enemy::Gargoyle,
        Enemy::Wraith,
        Enemy::Healer,
        Enemy::GiantScorpion,
        Enemy::Nightblade,
        Enemy::DaedraSeducer,
        Enemy::GiantScorpion,
        Enemy::Daedroth,
        Enemy::DaedraLord,
    ],
    // Cemetery
    [
        Enemy::Rat,
        Enemy::Rat,
        Enemy::GiantBat,
        Enemy::Rat,
        Enemy::Thief,
        Enemy::GiantBat,
        Enemy::SkeletalWarrior,
        Enemy::GiantBat,
        Enemy::Orc,
        Enemy::Archer,
        Enemy::Orc,
        Enemy::Imp,
        Enemy::Orc,
        Enemy::Imp,
        Enemy::Zombie,
        Enemy::Mummy,
        Enemy::Bard,
        Enemy::Rogue,
        Enemy::Rat,
        Enemy::Rat,
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dungeon_types_round_trip() {
        assert_eq!(DungeonType::from_raw(1).unwrap().name(), "Orc Stronghold");
        assert_eq!(DungeonType::from_raw(255), None);
        assert_eq!(DungeonType::from_name("vampire haunt"), DungeonType::from_raw(8));
    }

    #[test]
    fn chances_add_up_for_every_type_and_level() {
        for dungeon_type in DungeonType::all() {
            for level in [1, 3, 5, 6, 10, 16, 17, 30] {
                for table in [EncounterTable::Dungeon(dungeon_type), EncounterTable::Underwater] {
                    let total: f32 = enemy_chances(table, level).iter().map(|chance| chance.chance).sum();
                    assert!((total - 1.0).abs() < 1e-5, "{table:?} at level {level}: {total}");
                }
            }
        }
    }

    #[test]
    fn low_levels_use_the_start_of_the_table() {
        // At level 1 every roll picks from the first six entries of the crypt table:
        // Rat, Skeletal Warrior, Giant Bat, Rat, Skeletal Warrior, Giant Bat.
        let crypt = EncounterTable::Dungeon(DungeonType::from_name("Crypt").unwrap());
        let names: Vec<&str> = enemy_chances(crypt, 1).iter().map(|chance| chance.enemy.name()).collect();
        assert_eq!(names.len(), 3);
        assert!(
            names.contains(&"Rat") && names.contains(&"Skeletal Warrior") && names.contains(&"Giant Bat")
        );
        // High levels reach the end of the table, where the crypt's lich is.
        assert!(enemy_chances(crypt, 30).iter().any(|chance| chance.enemy == Enemy::Lich));
        // Above level 5 even the rarest roll stops at level + 3, so a level 10 player meets no lich.
        assert!(!enemy_chances(crypt, 10).iter().any(|chance| chance.enemy == Enemy::Lich));
    }

    #[test]
    fn underwater_markers_use_their_own_table() {
        let names: Vec<&str> =
            enemy_chances(EncounterTable::Underwater, 1).iter().map(|chance| chance.enemy.name()).collect();
        assert_eq!(names, ["Slaughterfish", "Skeletal Warrior", "Dreugh"]);
    }

    #[test]
    fn enemies_keep_their_game_ids() {
        assert_eq!(Enemy::from_mobile_id(0), Some(Enemy::Rat));
        assert_eq!(Enemy::from_mobile_id(12), Some(Enemy::OrcSergeant));
        assert_eq!(Enemy::from_mobile_id(128), Some(Enemy::Mage));
        assert_eq!(Enemy::from_mobile_id(146).map(Enemy::name), Some("City Watch"));
        assert_eq!(Enemy::from_mobile_id(99), None);
        assert!(Enemy::Knight.is_human() && !Enemy::Orc.is_human());
    }
}
