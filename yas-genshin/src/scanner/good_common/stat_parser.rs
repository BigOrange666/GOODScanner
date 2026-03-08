use lazy_static::lazy_static;
use regex::Regex;

/// Result of parsing a stat from OCR text
#[derive(Debug, Clone)]
pub struct ParsedStat {
    pub key: String,
    pub value: f64,
    pub inactive: bool,
}

/// Stat key entry: either a simple string key or flat/percent variants
enum StatKeyEntry {
    Simple(&'static str),
    FlatPercent { flat: &'static str, percent: &'static str },
}

/// Chinese stat name → GOOD stat key mapping
/// For HP/ATK/DEF, flat vs percent is determined by presence of "%" in OCR text
const STAT_KEY_ENTRIES: &[(&str, StatKeyEntry)] = &[
    ("\u{751F}\u{547D}\u{503C}", StatKeyEntry::FlatPercent { flat: "hp", percent: "hp_" }),           // 生命值
    ("\u{653B}\u{51FB}\u{529B}", StatKeyEntry::FlatPercent { flat: "atk", percent: "atk_" }),          // 攻击力
    ("\u{9632}\u{5FA1}\u{529B}", StatKeyEntry::FlatPercent { flat: "def", percent: "def_" }),          // 防御力
    ("\u{5143}\u{7D20}\u{7CBE}\u{901A}", StatKeyEntry::Simple("eleMas")),                              // 元素精通
    ("\u{5143}\u{7D20}\u{5145}\u{80FD}\u{6548}\u{7387}", StatKeyEntry::Simple("enerRech_")),           // 元素充能效率
    ("\u{66B4}\u{51FB}\u{7387}", StatKeyEntry::Simple("critRate_")),                                    // 暴击率
    ("\u{66B4}\u{51FB}\u{4F24}\u{5BB3}", StatKeyEntry::Simple("critDMG_")),                            // 暴击伤害
    ("\u{6CBB}\u{7597}\u{52A0}\u{6210}", StatKeyEntry::Simple("heal_")),                               // 治疗加成
    ("\u{7269}\u{7406}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("physical_dmg_")),       // 物理伤害加成
    ("\u{706B}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("pyro_dmg_")),   // 火元素伤害加成
    ("\u{96F7}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("electro_dmg_")),// 雷元素伤害加成
    ("\u{6C34}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("hydro_dmg_")),  // 水元素伤害加成
    ("\u{8349}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("dendro_dmg_")), // 草元素伤害加成
    ("\u{98CE}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("anemo_dmg_")),  // 风元素伤害加成
    ("\u{5CA9}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("geo_dmg_")),    // 岩元素伤害加成
    ("\u{51B0}\u{5143}\u{7D20}\u{4F24}\u{5BB3}\u{52A0}\u{6210}", StatKeyEntry::Simple("cryo_dmg_")),   // 冰元素伤害加成
];

lazy_static! {
    /// Stat names ordered by length descending for greedy matching
    static ref STAT_NAMES: Vec<&'static str> = {
        let mut names: Vec<&str> = STAT_KEY_ENTRIES.iter().map(|(name, _)| *name).collect();
        names.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()));
        names
    };

    static ref NUM_REGEX: Regex = Regex::new(r"[+\s]([\d]+\.?\d*)").unwrap();
}

/// Artifact slot mapping: Chinese slot name → GOOD slot key
pub const SLOT_KEY_MAP: &[(&str, &str)] = &[
    ("\u{751F}\u{4E4B}\u{82B1}", "flower"),  // 生之花
    ("\u{6B7B}\u{4E4B}\u{7FBD}", "plume"),   // 死之羽
    ("\u{65F6}\u{4E4B}\u{6C99}", "sands"),   // 时之沙
    ("\u{7A7A}\u{4E4B}\u{676F}", "goblet"),  // 空之杯
    ("\u{7406}\u{4E4B}\u{51A0}", "circlet"), // 理之冠
];

/// Parse a stat from OCR text.
///
/// Returns the GOOD stat key, numeric value, and whether the stat is inactive (待激活).
///
/// Port of `parseStatFromText()` from GOODScanner/lib/constants.js
pub fn parse_stat_from_text(text: &str) -> Option<ParsedStat> {
    if text.is_empty() {
        return None;
    }

    let text = text.replace(',', "");
    let text = text.trim();

    for &stat_name in STAT_NAMES.iter() {
        if !text.contains(stat_name) {
            continue;
        }

        let is_inactive = text.contains("\u{5F85}\u{6FC0}\u{6D3B}"); // 待激活
        let has_percent = text.contains('%');

        // Extract numeric value
        let value = if let Some(caps) = NUM_REGEX.captures(text) {
            caps[1].parse::<f64>().unwrap_or(0.0)
        } else {
            0.0
        };

        // Look up the key
        let key = STAT_KEY_ENTRIES
            .iter()
            .find(|(name, _)| *name == stat_name)
            .map(|(_, entry)| match entry {
                StatKeyEntry::Simple(k) => k.to_string(),
                StatKeyEntry::FlatPercent { flat, percent } => {
                    if has_percent {
                        percent.to_string()
                    } else {
                        flat.to_string()
                    }
                }
            })?;

        return Some(ParsedStat {
            key,
            value: if is_inactive { 0.0 } else { value },
            inactive: is_inactive,
        });
    }

    None
}

/// Match OCR text against the slot key map.
///
/// Returns the GOOD slot key (e.g., "flower", "plume") or None.
pub fn match_slot_key(text: &str) -> Option<&'static str> {
    for &(cn_name, key) in SLOT_KEY_MAP {
        if text.contains(cn_name) {
            return Some(key);
        }
    }
    None
}

/// Derive ascension phase from level and ascended status.
///
/// Ascension boundaries: 20→0, 40→1, 50→2, 60→3, 70→4, 80→5, 90→6
/// When level equals a boundary, `ascended` determines if the character/weapon
/// has been ascended past that boundary.
///
/// Port of `levelToAscension()` from GOODScanner/lib/constants.js
pub fn level_to_ascension(level: i32, ascended: bool) -> i32 {
    let thresholds = [20, 40, 50, 60, 70, 80];
    for (i, &threshold) in thresholds.iter().enumerate() {
        if level < threshold {
            return i as i32;
        }
        if level == threshold {
            return if ascended { i as i32 + 1 } else { i as i32 };
        }
    }
    6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_to_ascension() {
        assert_eq!(level_to_ascension(1, false), 0);
        assert_eq!(level_to_ascension(20, false), 0);
        assert_eq!(level_to_ascension(20, true), 1);
        assert_eq!(level_to_ascension(40, true), 2);
        assert_eq!(level_to_ascension(50, false), 2);
        assert_eq!(level_to_ascension(80, true), 6);
        assert_eq!(level_to_ascension(90, false), 6);
    }

    #[test]
    fn test_parse_stat_percent() {
        let result = parse_stat_from_text("\u{653B}\u{51FB}\u{529B}+46.6%"); // 攻击力+46.6%
        assert!(result.is_some());
        let stat = result.unwrap();
        assert_eq!(stat.key, "atk_");
        assert!((stat.value - 46.6).abs() < 0.01);
        assert!(!stat.inactive);
    }

    #[test]
    fn test_parse_stat_flat() {
        let result = parse_stat_from_text("\u{751F}\u{547D}\u{503C}+4780"); // 生命值+4780
        assert!(result.is_some());
        let stat = result.unwrap();
        assert_eq!(stat.key, "hp");
        assert!((stat.value - 4780.0).abs() < 0.01);
    }

    #[test]
    fn test_match_slot_key() {
        assert_eq!(match_slot_key("\u{751F}\u{4E4B}\u{82B1}"), Some("flower")); // 生之花
        assert_eq!(match_slot_key("\u{7406}\u{4E4B}\u{51A0}"), Some("circlet")); // 理之冠
        assert_eq!(match_slot_key("random"), None);
    }
}
