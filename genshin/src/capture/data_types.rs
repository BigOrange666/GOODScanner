/// Types for deserializing `data_cache.json` (irminsul/anime-game-data format).
///
/// These replicate the `Database` struct from the `anime-game-data` crate exactly,
/// so that `data_cache.json` files are interchangeable between yas and irminsul.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct DataCache {
    pub version: u32,
    pub git_hash: String,
    #[serde(default)]
    pub affix_map: HashMap<u32, Affix>,
    #[serde(default)]
    pub artifact_map: HashMap<u32, Artifact>,
    #[serde(default)]
    pub character_map: HashMap<u32, String>,
    #[serde(default)]
    pub material_map: HashMap<u32, String>,
    #[serde(default)]
    pub property_map: HashMap<u32, Property>,
    #[serde(default)]
    pub set_map: HashMap<u32, String>,
    #[serde(default)]
    pub skill_type_map: HashMap<u32, SkillType>,
    #[serde(default)]
    pub weapon_map: HashMap<u32, Weapon>,
}

impl DataCache {
    pub fn get_affix(&self, id: u32) -> Option<&Affix> {
        self.affix_map.get(&id)
    }

    pub fn get_artifact(&self, id: u32) -> Option<&Artifact> {
        self.artifact_map.get(&id)
    }

    pub fn get_character(&self, id: u32) -> Option<&String> {
        self.character_map.get(&id)
    }

    pub fn get_property(&self, id: u32) -> Option<&Property> {
        self.property_map.get(&id)
    }

    pub fn get_set(&self, id: u32) -> Option<&String> {
        self.set_map.get(&id)
    }

    pub fn get_skill_type(&self, id: u32) -> Option<&SkillType> {
        self.skill_type_map.get(&id)
    }

    pub fn get_weapon(&self, id: u32) -> Option<&Weapon> {
        self.weapon_map.get(&id)
    }
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct Affix {
    pub property: Property,
    pub value: f64,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Artifact {
    pub set: String,
    pub slot: ArtifactSlot,
    pub rarity: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ArtifactSlot {
    Flower,
    Plume,
    Sands,
    Goblet,
    Circlet,
}

impl ArtifactSlot {
    pub fn good_name(&self) -> &str {
        match self {
            ArtifactSlot::Flower => "flower",
            ArtifactSlot::Plume => "plume",
            ArtifactSlot::Sands => "sands",
            ArtifactSlot::Goblet => "goblet",
            ArtifactSlot::Circlet => "circlet",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Property {
    Hp,
    HpPercent,
    Attack,
    AttackPercent,
    Defense,
    DefensePercent,
    ElementalMastery,
    EnergyRecharge,
    Healing,
    CritRate,
    CritDamage,
    PhysicalDamage,
    AnemoDamage,
    GeoDamage,
    ElectroDamage,
    HydroDamage,
    PyroDamage,
    CryoDamage,
    DendroDamage,
}

impl Property {
    pub fn good_name(&self) -> &str {
        match self {
            Property::Hp => "hp",
            Property::HpPercent => "hp_",
            Property::Attack => "atk",
            Property::AttackPercent => "atk_",
            Property::Defense => "def",
            Property::DefensePercent => "def_",
            Property::ElementalMastery => "eleMas",
            Property::EnergyRecharge => "enerRech_",
            Property::Healing => "heal_",
            Property::CritRate => "critRate_",
            Property::CritDamage => "critDMG_",
            Property::PhysicalDamage => "physical_dmg_",
            Property::AnemoDamage => "anemo_dmg_",
            Property::GeoDamage => "geo_dmg_",
            Property::ElectroDamage => "electro_dmg_",
            Property::HydroDamage => "hydro_dmg_",
            Property::PyroDamage => "pyro_dmg_",
            Property::CryoDamage => "cryo_dmg_",
            Property::DendroDamage => "dendro_dmg_",
        }
    }

    pub fn is_percentage(&self) -> bool {
        matches!(
            self,
            Property::HpPercent
                | Property::AttackPercent
                | Property::DefensePercent
                | Property::EnergyRecharge
                | Property::Healing
                | Property::CritRate
                | Property::CritDamage
                | Property::PhysicalDamage
                | Property::AnemoDamage
                | Property::GeoDamage
                | Property::ElectroDamage
                | Property::HydroDamage
                | Property::PyroDamage
                | Property::CryoDamage
                | Property::DendroDamage
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SkillType {
    Auto,
    Skill,
    Burst,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Weapon {
    pub name: String,
    pub rarity: u32,
}

/// Convert an English name to a GOOD PascalCase key.
///
/// e.g. "Primordial Jade Cutter" → "PrimordialJadeCutter"
///      "Furina" → "Furina"
///
/// Replicates `to_good_key` from irminsul.
pub fn to_good_key(value: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            if capitalize_next {
                result.extend(c.to_uppercase());
                capitalize_next = false;
            } else {
                result.push(c);
            }
        } else if c == ' ' {
            capitalize_next = true;
        }
    }

    result
}
