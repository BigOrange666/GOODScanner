use std::rc::Rc;
use std::time::SystemTime;

use anyhow::{bail, Result};
use image::RgbImage;
use log::{error, info, warn};
use regex::Regex;

use yas::capture::{Capturer, GenericCapturer};
use yas::game_info::GameInfo;
use yas::ocr::{ImageToText, PPOCRModel};
use yas::system_control::SystemControl;
use yas::utils;

use super::GoodCharacterScannerConfig;
use crate::scanner::good_common::constants::*;
use crate::scanner::good_common::coord_scaler::CoordScaler;
use crate::scanner::good_common::fuzzy_match::fuzzy_match_map;
use crate::scanner::good_common::mappings::MappingManager;
use crate::scanner::good_common::models::{GoodCharacter, GoodTalent};
use crate::scanner::good_common::navigation;
use crate::scanner::good_common::stat_parser::level_to_ascension;

/// Character scanner ported from GOODScanner/lib/character_scanner.js.
///
/// Uses binary-search constellation detection (max 3 clicks),
/// talent adjustments (Tartaglia -1, C3/C5 bonus subtraction),
/// and alternating scan direction for tab optimization.
pub struct GoodCharacterScanner {
    config: GoodCharacterScannerConfig,
    game_info: GameInfo,
    scaler: CoordScaler,
    capturer: Rc<dyn Capturer<RgbImage>>,
    system_control: SystemControl,
    ocr_model: Box<dyn ImageToText<RgbImage> + Send>,
    mappings: Rc<MappingManager>,
}

impl GoodCharacterScanner {
    fn get_image_to_text(backend: &str) -> Result<Box<dyn ImageToText<RgbImage> + Send>> {
        match backend.to_lowercase().as_str() {
            "paddlev3" | "ppocrv3" => {
                let model_bytes = include_bytes!("../character_scanner/models/ch_PP-OCRv3_rec_infer.onnx");
                let dict_str = include_str!("../character_scanner/models/ppocr_keys_v1.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.trim().to_string()).collect();
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            }
            _ => {
                // Default to PPOCRv5
                let model_bytes = include_bytes!("../character_scanner/models/PP-OCRv5_mobile_rec.onnx");
                let dict_str = include_str!("../character_scanner/models/ppocrv5_dict.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.trim().to_string()).collect();
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            }
        }
    }

    pub fn new(
        config: GoodCharacterScannerConfig,
        game_info: GameInfo,
        mappings: Rc<MappingManager>,
    ) -> Result<Self> {
        let ocr_model = Self::get_image_to_text(&config.ocr_backend)?;
        let window_size = game_info.window.to_rect_usize().size();
        let scaler = CoordScaler::new(window_size.width as u32, window_size.height as u32);

        Ok(Self {
            config,
            game_info,
            scaler,
            capturer: Rc::new(GenericCapturer::new()?),
            system_control: SystemControl::new(),
            ocr_model,
            mappings,
        })
    }
}

impl GoodCharacterScanner {
    /// OCR a region in base 1920x1080 coordinates
    fn ocr_rect(&self, rect: (f64, f64, f64, f64)) -> Result<String> {
        navigation::ocr_region(
            &self.game_info,
            &self.scaler,
            self.capturer.as_ref(),
            self.ocr_model.as_ref(),
            rect,
        )
    }

    /// Click at base coordinates
    fn click(&mut self, base_x: f64, base_y: f64) {
        navigation::click_at(
            &self.game_info,
            &self.scaler,
            &mut self.system_control,
            base_x,
            base_y,
        );
    }

    /// Parse character name and element from OCR text.
    /// Text format: "Element/CharacterName" (e.g., "冰/神里绫华")
    ///
    /// Port of `parseCharacterNameAndElement()` from character_scanner.js
    fn parse_name_and_element(&self, text: &str) -> (Option<String>, Option<String>) {
        if text.is_empty() {
            return (None, None);
        }

        if let Some(idx) = text.find('/') {
            let element = text[..idx].trim().to_string();
            let raw_name: String = text[idx + 1..]
                .chars()
                .filter(|c| {
                    matches!(*c, '\u{4E00}'..='\u{9FFF}' | '\u{300C}' | '\u{300D}' | 'a'..='z' | 'A'..='Z' | '0'..='9')
                })
                .collect();
            let name = fuzzy_match_map(&raw_name, &self.mappings.character_name_map);
            (name, Some(element))
        } else {
            let name = fuzzy_match_map(text, &self.mappings.character_name_map);
            (name, None)
        }
    }

    /// OCR read character name and element, with one retry.
    ///
    /// Port of `readCharacterNameAndElement()` from character_scanner.js
    fn read_name_and_element(&self) -> Result<(Option<String>, Option<String>, String)> {
        let text = self.ocr_rect(CHAR_NAME_RECT)?;
        let (name, element) = self.parse_name_and_element(&text);

        if name.is_some() {
            return Ok((name, element, text));
        }

        warn!("[character] first name match failed: \u{300C}{}\u{300D}, retrying...", text);
        utils::sleep(1000);

        let text2 = self.ocr_rect(CHAR_NAME_RECT)?;
        let (name2, element2) = self.parse_name_and_element(&text2);
        if name2.is_none() {
            warn!("[character] second name match failed: \u{300C}{}\u{300D}", text2);
        }
        Ok((name2, element2, text2))
    }

    /// OCR read character level, returns (level, ascended).
    ///
    /// Port of `readCharacterLevel()` from character_scanner.js
    fn read_level(&self) -> Result<(i32, bool)> {
        let text = self.ocr_rect(CHAR_LEVEL_RECT)?;

        let re = Regex::new(r"(\d+)\s*/\s*(\d+)")?;
        if let Some(caps) = re.captures(&text) {
            let level: i32 = caps[1].parse().unwrap_or(1);
            let raw_max: i32 = caps[2].parse().unwrap_or(20);
            // Round max level to nearest 10
            let max_level = ((raw_max as f64 / 10.0).round() * 10.0) as i32;
            let ascended = level >= 20 && level < max_level;
            Ok((level, ascended))
        } else {
            let level = navigation::parse_number_from_text(&text);
            Ok((if level > 0 { level } else { 1 }, false))
        }
    }

    /// Click a constellation node and check if it's activated via OCR.
    ///
    /// Port of `isConstellationActivated()` from character_scanner.js
    fn is_constellation_activated(&mut self, c_index: usize, is_first_click: bool) -> Result<bool> {
        let click_y = CHAR_CONSTELLATION_Y_BASE + c_index as f64 * CHAR_CONSTELLATION_Y_STEP;
        self.click(CHAR_CONSTELLATION_X, click_y);

        let delay = if is_first_click {
            self.config.tab_delay
        } else {
            self.config.tab_delay / 2
        };
        utils::sleep(delay as u32);

        let text = self.ocr_rect(CHAR_CONSTELLATION_ACTIVATE_RECT)?;
        // "已激活" means "Activated"
        Ok(text.contains("\u{5DF2}\u{6FC0}\u{6D3B}"))
    }

    /// Binary-search constellation count (max 3 clicks).
    ///
    /// Algorithm:
    /// - Check C3: if inactive → check C2 → if inactive → check C1
    /// - Check C3: if active → check C6 → if active → 6
    ///                                   → if inactive → check C4 → check C5
    ///
    /// Special: Geo element C5 recheck for C6 due to animation interference.
    ///
    /// Port of `readConstellationCount()` from character_scanner.js
    fn read_constellation_count(
        &mut self,
        character_name: &str,
        element: &Option<String>,
    ) -> Result<i32> {
        // Skip characters without constellations
        if NO_CONSTELLATION_CHARACTERS.contains(&character_name) {
            return Ok(0);
        }

        self.click(CHAR_TAB_CONSTELLATION.0, CHAR_TAB_CONSTELLATION.1);
        utils::sleep(self.config.tab_delay as u32);

        let constellation;

        let c3 = self.is_constellation_activated(2, true)?;
        if !c3 {
            let c2 = self.is_constellation_activated(1, false)?;
            if !c2 {
                let c1 = self.is_constellation_activated(0, false)?;
                constellation = if c1 { 1 } else { 0 };
            } else {
                constellation = 2;
            }
        } else {
            let c6 = self.is_constellation_activated(5, false)?;
            if c6 {
                constellation = 6;
            } else {
                let c4 = self.is_constellation_activated(3, false)?;
                if !c4 {
                    constellation = 3;
                } else {
                    let c5 = self.is_constellation_activated(4, false)?;
                    constellation = if c5 { 5 } else { 4 };
                }
            }
        }

        // Geo element: background animation may interfere with C5→C6 OCR
        let mut final_constellation = constellation;
        if constellation == 5 {
            if let Some(elem) = element {
                if elem.contains('\u{5CA9}') {
                    // 岩 = Geo
                    let c6_recheck = self.is_constellation_activated(5, false)?;
                    if c6_recheck {
                        final_constellation = 6;
                        warn!("[character] Geo C6 recheck passed, corrected to C6");
                    }
                }
            }
        }

        self.system_control.key_press(enigo::Key::Escape).unwrap();
        utils::sleep(self.config.tab_delay as u32);

        Ok(final_constellation)
    }

    /// Parse "Lv.X" format from talent overview text.
    ///
    /// Port of `parseLvText()` from character_scanner.js
    fn parse_lv_text(text: &str) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let re = Regex::new(r"(?i)[Ll][Vv]\.?\s*(\d{1,2})").unwrap();
        if let Some(caps) = re.captures(text) {
            let lv: i32 = caps[1].parse().unwrap_or(0);
            if (1..=15).contains(&lv) {
                return lv;
            }
        }
        0
    }

    /// Read a single talent level by clicking the detail view.
    ///
    /// Port of `readTalentByClick()` from character_scanner.js
    fn read_talent_by_click(&mut self, talent_index: usize, is_first: bool) -> Result<i32> {
        let click_y = CHAR_TALENT_FIRST_Y + talent_index as f64 * CHAR_TALENT_OFFSET_Y;
        self.click(CHAR_TALENT_CLICK_X, click_y);

        let delay = if is_first {
            self.config.tab_delay
        } else {
            self.config.tab_delay / 2
        };
        utils::sleep(delay as u32);

        let text = self.ocr_rect(CHAR_TALENT_LEVEL_RECT)?;
        let re = Regex::new(r"(\d+)")?;
        if let Some(caps) = re.captures(&text) {
            let v: i32 = caps[1].parse().unwrap_or(1);
            if (1..=15).contains(&v) {
                return Ok(v);
            }
        }
        Ok(1)
    }

    /// Read all three talent levels using overview OCR first, with click fallback.
    ///
    /// Port of `readTalentLevels()` from character_scanner.js
    fn read_talent_levels(
        &mut self,
        character_name: &str,
        skip_tab: bool,
    ) -> Result<(i32, i32, i32)> {
        if !skip_tab {
            self.click(CHAR_TAB_TALENTS.0, CHAR_TAB_TALENTS.1);
            utils::sleep(self.config.tab_delay as u32);
        }

        let has_special = SPECIAL_BURST_CHARACTERS.contains(&character_name);

        // Try overview OCR first
        let auto_lv = Self::parse_lv_text(&self.ocr_rect(CHAR_TALENT_OVERVIEW_AUTO)?);
        let skill_lv = Self::parse_lv_text(&self.ocr_rect(CHAR_TALENT_OVERVIEW_SKILL)?);
        let burst_rect = if has_special {
            CHAR_TALENT_OVERVIEW_BURST_SPECIAL
        } else {
            CHAR_TALENT_OVERVIEW_BURST
        };
        let burst_lv = Self::parse_lv_text(&self.ocr_rect(burst_rect)?);

        let mut auto = if auto_lv > 0 { auto_lv } else { 1 };
        let mut skill = if skill_lv > 0 { skill_lv } else { 1 };
        let mut burst = if burst_lv > 0 { burst_lv } else { 1 };

        // Fallback to click-detail for any that failed
        let need_click = auto_lv == 0 || skill_lv == 0 || burst_lv == 0;
        if need_click {
            let mut missing = Vec::new();
            if auto_lv == 0 { missing.push("auto"); }
            if skill_lv == 0 { missing.push("skill"); }
            if burst_lv == 0 { missing.push("burst"); }
            warn!(
                "[character] talent overview failed for: {}, using click fallback",
                missing.join("/")
            );

            let mut is_first = true;
            if auto_lv == 0 {
                auto = self.read_talent_by_click(0, is_first)?;
                is_first = false;
            }
            if skill_lv == 0 {
                skill = self.read_talent_by_click(1, is_first)?;
                is_first = false;
            }
            if burst_lv == 0 {
                let burst_index = if has_special { 3 } else { 2 };
                burst = self.read_talent_by_click(burst_index, is_first)?;
            }
            self.system_control.key_press(enigo::Key::Escape).unwrap();
            utils::sleep(500);
        }

        Ok((auto, skill, burst))
    }

    /// Scan a single character.
    ///
    /// `first_name`: the first character's key for loop detection (None on first scan).
    /// `reverse`: if true, scan in talents→constellation→attributes order.
    ///
    /// Returns `Ok(Some(character))` on success, `Ok(None)` to skip,
    /// or a special marker via `first_name` match detection.
    ///
    /// Port of `scanSingleCharacter()` from character_scanner.js
    fn scan_single_character(
        &mut self,
        first_name: &Option<String>,
        reverse: bool,
    ) -> Result<Option<GoodCharacter>> {
        // Name and element are visible from any tab
        let (name, element, raw_text) = self.read_name_and_element()?;

        let name = match name {
            Some(n) => n,
            None => {
                if self.config.continue_on_failure {
                    warn!("[character] cannot identify: \u{300C}{}\u{300D}, skipping", raw_text);
                    return Ok(None);
                }
                bail!("Cannot identify character: \u{300C}{}\u{300D}", raw_text);
            }
        };

        // Loop detection
        if let Some(first) = first_name {
            if &name == first {
                return Err(anyhow::anyhow!("_repeat"));
            }
        }

        let level_info;
        let constellation;
        let talents;

        if !reverse {
            // Forward: attributes → constellation → talents (already on attributes tab)
            level_info = self.read_level()?;
            constellation = self.read_constellation_count(&name, &element)?;
            talents = self.read_talent_levels(&name, false)?;
        } else {
            // Reverse: talents → constellation → attributes (already on talents tab)
            talents = self.read_talent_levels(&name, true)?;
            constellation = self.read_constellation_count(&name, &element)?;
            self.click(CHAR_TAB_ATTRIBUTES.0, CHAR_TAB_ATTRIBUTES.1);
            utils::sleep(self.config.tab_delay as u32);
            level_info = self.read_level()?;
        }

        let (level, ascended) = level_info;
        let ascension = level_to_ascension(level, ascended);

        let mut auto = talents.0;
        let mut skill = talents.1;
        let mut burst = talents.2;

        // Tartaglia innate talent: auto +1 bonus
        if name == TARTAGLIA_KEY {
            auto = (auto - 1).max(1);
        }

        // Subtract constellation talent bonuses (C3/C5 each add +3)
        if let Some(bonus) = self.mappings.character_const_bonus.get(&name) {
            if constellation >= 3 {
                if let Some(ref c3_type) = bonus.c3 {
                    match c3_type.as_str() {
                        "A" => auto = (auto - 3).max(1),
                        "E" => skill = (skill - 3).max(1),
                        "Q" => burst = (burst - 3).max(1),
                        _ => {}
                    }
                }
            }
            if constellation >= 5 {
                if let Some(ref c5_type) = bonus.c5 {
                    match c5_type.as_str() {
                        "A" => auto = (auto - 3).max(1),
                        "E" => skill = (skill - 3).max(1),
                        "Q" => burst = (burst - 3).max(1),
                        _ => {}
                    }
                }
            }
        }

        Ok(Some(GoodCharacter {
            key: name,
            level,
            constellation,
            ascension,
            talent: GoodTalent {
                auto,
                skill,
                burst,
            },
        }))
    }

    /// Scan all characters by iterating through the character list.
    ///
    /// Alternates scan direction (reverse flag) each character for tab optimization.
    /// Detects loop completion when the first character is seen again.
    ///
    /// Port of `scanAllCharacters()` from character_scanner.js
    pub fn scan(&mut self) -> Result<Vec<GoodCharacter>> {
        info!("[character] starting scan...");
        let now = SystemTime::now();

        // Open character screen
        navigation::open_character_screen(&mut self.system_control, self.config.open_delay);

        let mut characters: Vec<GoodCharacter> = Vec::new();
        let mut first_name: Option<String> = None;
        let mut viewed_count = 0;
        let mut reverse = false;

        loop {
            if utils::is_rmb_down() {
                info!("[character] user interrupted scan");
                break;
            }

            let result = self.scan_single_character(&first_name, reverse);

            match result {
                Ok(Some(character)) => {
                    if first_name.is_none() {
                        first_name = Some(character.key.clone());
                    }
                    if self.config.log_progress {
                        info!(
                            "[character] {} Lv.{} C{} {}/{}/{}",
                            character.key, character.level, character.constellation,
                            character.talent.auto, character.talent.skill, character.talent.burst
                        );
                    }
                    characters.push(character);
                }
                Ok(None) => {
                    // Skipped (continue_on_failure)
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg == "_repeat" {
                        info!("[character] loop detected, scan complete");
                        break;
                    }
                    error!("[character] scan error: {}", e);
                    if !self.config.continue_on_failure {
                        break;
                    }
                }
            }

            viewed_count += 1;
            if viewed_count > 3 && characters.is_empty() {
                error!("[character] viewed {} but no results, stopping", viewed_count);
                break;
            }

            // Navigate to next character
            self.click(CHAR_NEXT_POS.0, CHAR_NEXT_POS.1);
            utils::sleep(self.config.tab_delay as u32);
            reverse = !reverse;
        }

        // Close character screen
        self.system_control.key_press(enigo::Key::Escape).unwrap();
        utils::sleep(500);

        info!(
            "[character] complete, {} characters scanned in {:?}",
            characters.len(),
            now.elapsed().unwrap_or_default()
        );

        Ok(characters)
    }
}
