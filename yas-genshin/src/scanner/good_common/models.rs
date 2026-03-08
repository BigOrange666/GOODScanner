use serde::Serialize;

/// GOOD v3 character export
#[derive(Debug, Clone, Serialize)]
pub struct GoodCharacter {
    pub key: String,
    pub level: i32,
    pub constellation: i32,
    pub ascension: i32,
    pub talent: GoodTalent,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoodTalent {
    pub auto: i32,
    pub skill: i32,
    pub burst: i32,
}

/// GOOD v3 weapon export
#[derive(Debug, Clone, Serialize)]
pub struct GoodWeapon {
    pub key: String,
    pub level: i32,
    pub ascension: i32,
    pub refinement: i32,
    pub rarity: i32,
    pub location: String,
    pub lock: bool,
}

/// GOOD v3 artifact export
#[derive(Debug, Clone, Serialize)]
pub struct GoodArtifact {
    #[serde(rename = "setKey")]
    pub set_key: String,
    #[serde(rename = "slotKey")]
    pub slot_key: String,
    pub level: i32,
    pub rarity: i32,
    #[serde(rename = "mainStatKey")]
    pub main_stat_key: String,
    pub substats: Vec<GoodSubStat>,
    pub location: String,
    pub lock: bool,
    #[serde(rename = "astralMark")]
    pub astral_mark: bool,
    #[serde(rename = "elixirCrafted")]
    pub elixir_crafted: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", rename = "unactivatedSubstats")]
    pub unactivated_substats: Vec<GoodSubStat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoodSubStat {
    pub key: String,
    pub value: f64,
}

/// GOOD v3 full export
#[derive(Debug, Clone, Serialize)]
pub struct GoodExport {
    pub format: String,
    pub version: u32,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characters: Option<Vec<GoodCharacter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weapons: Option<Vec<GoodWeapon>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<GoodArtifact>>,
}

impl GoodExport {
    pub fn new(
        characters: Option<Vec<GoodCharacter>>,
        weapons: Option<Vec<GoodWeapon>>,
        artifacts: Option<Vec<GoodArtifact>>,
    ) -> Self {
        Self {
            format: "GOOD".to_string(),
            version: 3,
            source: "yas-GOODScanner".to_string(),
            characters,
            weapons,
            artifacts,
        }
    }
}
