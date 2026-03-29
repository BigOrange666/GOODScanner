use std::collections::HashMap;
use super::fuzzy_match::fuzzy_match_map;

/// Parse equipped character from equip text.
///
/// The OCR region captures text like "CharName已装备" with possible noise
/// prefix chars from card decorations (c, Y, ca, emojis, etc).
/// Also handles truncated "已装" when the region clips the right side.
pub fn parse_equip_location(text: &str, char_map: &HashMap<String, String>) -> String {
    // Check for "已装备" or truncated "已装"
    let equip_marker = if text.contains("\u{5DF2}\u{88C5}\u{5907}") {
        Some("\u{5DF2}\u{88C5}\u{5907}") // 已装备
    } else if text.contains("\u{5DF2}\u{88C5}") {
        Some("\u{5DF2}\u{88C5}") // 已装 (truncated)
    } else {
        None
    };

    if let Some(marker) = equip_marker {
        let char_name = text
            .replace(marker, "")
            .replace(['\u{5907}', ':', '\u{FF1A}', ' '], "") // also strip stray 备
            .trim()
            .to_string();

        // Strip leading ASCII noise (c, Y, n, etc.) and emojis from OCR
        let cleaned: String = char_name
            .trim_start_matches(|c: char| c.is_ascii() || !c.is_alphanumeric())
            .to_string();

        for name in [&cleaned, &char_name] {
            if !name.is_empty() {
                if let Some(key) = fuzzy_match_map(name, char_map) {
                    return key;
                }
            }
        }
    }
    String::new()
}
