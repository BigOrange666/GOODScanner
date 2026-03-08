use std::rc::Rc;
use std::time::SystemTime;

use anyhow::{bail, Result};
use image::{GenericImageView, RgbImage};
use log::{error, info, warn};
use regex::Regex;

use yas::capture::{Capturer, GenericCapturer};
use yas::game_info::GameInfo;
use yas::ocr::{ImageToText, PPOCRModel};
use yas::system_control::SystemControl;
use yas::utils;

use super::GoodWeaponScannerConfig;
use crate::scanner::good_common::constants::*;
use crate::scanner::good_common::coord_scaler::CoordScaler;
use crate::scanner::good_common::fuzzy_match::fuzzy_match_map;
use crate::scanner::good_common::mappings::MappingManager;
use crate::scanner::good_common::models::GoodWeapon;
use crate::scanner::good_common::navigation;
use crate::scanner::good_common::pixel_utils;
use crate::scanner::good_common::stat_parser::level_to_ascension;

/// Computed OCR regions for weapon card (at 1920x1080 base).
/// Port of the WEAPON_OCR calculation from GOODScanner/lib/weapon_scanner.js
struct WeaponOcrRegions {
    name: (f64, f64, f64, f64),
    level: (f64, f64, f64, f64),
    refinement: (f64, f64, f64, f64),
    equip: (f64, f64, f64, f64),
}

impl WeaponOcrRegions {
    fn new() -> Self {
        let card_x: f64 = 1307.0;
        let card_y: f64 = 119.0;
        let card_w: f64 = 494.0;
        let card_h: f64 = 841.0;

        Self {
            name: (card_x, card_y, card_w, (card_h * 0.07).round()),
            level: (
                card_x + (card_w * 0.060).round(),
                card_y + (card_h * 0.367).round(),
                (card_w * 0.262).round(),
                (card_h * 0.035).round(),
            ),
            refinement: (
                card_x + (card_w * 0.058).round(),
                card_y + (card_h * 0.417).round(),
                (card_w * 0.25).round(),
                (card_h * 0.038).round(),
            ),
            equip: (
                card_x + (card_w * 0.10).round(),
                card_y + (card_h * 0.935).round(),
                (card_w * 0.85).round(),
                (card_h * 0.06).round(),
            ),
        }
    }
}

/// Weapon scanner ported from GOODScanner/lib/weapon_scanner.js.
///
/// Scans weapons from the backpack grid, detecting name/level/refinement/equip
/// via OCR on captured game images. Stops when low-tier weapons are reached.
pub struct GoodWeaponScanner {
    config: GoodWeaponScannerConfig,
    game_info: GameInfo,
    scaler: CoordScaler,
    capturer: Rc<dyn Capturer<RgbImage>>,
    system_control: SystemControl,
    ocr_model: Box<dyn ImageToText<RgbImage> + Send>,
    mappings: Rc<MappingManager>,
    ocr_regions: WeaponOcrRegions,
}

/// Additional forging material stop names not in the shared constants
const WEAPON_FORGING_STOP_NAMES: &[&str] = &[
    "\u{7CBE}\u{953B}\u{7528}\u{9B54}\u{77FF}", // 精锻用魔矿
    "\u{7CBE}\u{953B}\u{7528}\u{826F}\u{77FF}", // 精锻用良矿
    "\u{7CBE}\u{953B}\u{7528}\u{6742}\u{77FF}", // 精锻用杂矿
];

/// Result of scanning a single weapon: weapon data, stop signal, or skip
enum WeaponScanResult {
    Weapon(GoodWeapon),
    Stop,
    Skip,
}

impl GoodWeaponScanner {
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
        config: GoodWeaponScannerConfig,
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
            ocr_regions: WeaponOcrRegions::new(),
        })
    }
}

impl GoodWeaponScanner {
    /// OCR a sub-region of a captured game image.
    /// Crops the sub-region, converts to RgbImage, and runs OCR.
    fn ocr_image_region(
        &self,
        image: &RgbImage,
        rect: (f64, f64, f64, f64),
    ) -> Result<String> {
        let (bx, by, bw, bh) = rect;
        let x = self.scaler.x(bx) as u32;
        let y = self.scaler.y(by) as u32;
        let w = self.scaler.x(bw) as u32;
        let h = self.scaler.y(bh) as u32;

        // Clamp to image bounds
        let x = x.min(image.width().saturating_sub(1));
        let y = y.min(image.height().saturating_sub(1));
        let w = w.min(image.width().saturating_sub(x));
        let h = h.min(image.height().saturating_sub(y));

        if w == 0 || h == 0 {
            return Ok(String::new());
        }

        let sub = image.view(x, y, w, h).to_image();
        let text = self.ocr_model.image_to_text(&sub, false)?;
        Ok(text.trim().to_string())
    }

    /// Capture the full game window
    fn capture_game(&self) -> Result<RgbImage> {
        navigation::capture_game_region(&self.game_info, self.capturer.as_ref())
    }

    /// Scan a single weapon from a captured game image.
    ///
    /// Port of `scanSingleWeapon()` from GOODScanner/lib/weapon_scanner.js
    fn scan_single_weapon(&self, image: &RgbImage) -> Result<WeaponScanResult> {
        // OCR weapon name
        let name_text = self.ocr_image_region(image, self.ocr_regions.name)?;
        let weapon_key = fuzzy_match_map(&name_text, &self.mappings.weapon_name_map);

        if weapon_key.is_none() {
            // Check if it's a stop-signal weapon/material
            for &stop_name in WEAPON_STOP_NAMES.iter().chain(WEAPON_FORGING_STOP_NAMES.iter()) {
                if name_text.contains(stop_name) {
                    info!("[weapon] detected \u{300C}{}\u{300D}, stopping", stop_name);
                    return Ok(WeaponScanResult::Stop);
                }
            }

            if pixel_utils::detect_weapon_rarity(image, &self.scaler) <= 2 {
                info!("[weapon] detected low-star item, stopping");
                return Ok(WeaponScanResult::Stop);
            }

            if self.config.continue_on_failure {
                warn!("[weapon] cannot match: \u{300C}{}\u{300D}, skipping", name_text);
                return Ok(WeaponScanResult::Skip);
            }
            bail!("Cannot match weapon: \u{300C}{}\u{300D}", name_text);
        }

        let weapon_key = weapon_key.unwrap();

        // OCR level
        let level_text = self.ocr_image_region(image, self.ocr_regions.level)?;
        let (level, ascended) = Self::parse_weapon_level(&level_text);

        // OCR refinement
        let ref_text = self.ocr_image_region(image, self.ocr_regions.refinement)?;
        let refinement = Self::parse_refinement(&ref_text);

        // OCR equip status
        let equip_text = self.ocr_image_region(image, self.ocr_regions.equip)?;
        let location = self.parse_equip_location(&equip_text);

        // Pixel-based detections
        let rarity = pixel_utils::detect_weapon_rarity(image, &self.scaler);
        let lock = pixel_utils::detect_weapon_lock(image, &self.scaler);
        let ascension = level_to_ascension(level, ascended);

        Ok(WeaponScanResult::Weapon(GoodWeapon {
            key: weapon_key,
            level,
            ascension,
            refinement,
            rarity,
            location,
            lock,
        }))
    }

    /// Parse weapon level from "XX/YY" or "Lv.X" format.
    /// Returns (level, ascended).
    fn parse_weapon_level(text: &str) -> (i32, bool) {
        if text.is_empty() {
            return (1, false);
        }

        let slash_re = Regex::new(r"(\d+)\s*/\s*(\d+)").unwrap();
        if let Some(caps) = slash_re.captures(text) {
            let level: i32 = caps[1].parse().unwrap_or(1);
            let raw_max: i32 = caps[2].parse().unwrap_or(20);
            let max_level = ((raw_max as f64 / 10.0).round() * 10.0) as i32;
            let ascended = level >= 20 && level < max_level;
            return (level, ascended);
        }

        let lv_re = Regex::new(r"(?i)[Ll][Vv]\.?\s*(\d+)").unwrap();
        if let Some(caps) = lv_re.captures(text) {
            let level: i32 = caps[1].parse().unwrap_or(1);
            return (level, false);
        }

        let level = navigation::parse_number_from_text(text);
        (if level > 0 { level } else { 1 }, false)
    }

    /// Parse refinement from text.
    /// Tries: "精炼X" → "RX" → bare digit 1-5.
    ///
    /// Port of refinement parsing from weapon_scanner.js
    fn parse_refinement(text: &str) -> i32 {
        if text.is_empty() {
            return 1;
        }

        // Try "精炼X"
        let cn_re = Regex::new(r"\u{7CBE}\u{70BC}\s*(\d)").unwrap();
        if let Some(caps) = cn_re.captures(text) {
            return caps[1].parse().unwrap_or(1);
        }

        // Try "RX"
        let r_re = Regex::new(r"(?i)[Rr](\d)").unwrap();
        if let Some(caps) = r_re.captures(text) {
            return caps[1].parse().unwrap_or(1);
        }

        // Try bare digit
        let d_re = Regex::new(r"(\d)").unwrap();
        if let Some(caps) = d_re.captures(text) {
            let d: i32 = caps[1].parse().unwrap_or(0);
            if (1..=5).contains(&d) {
                return d;
            }
        }

        1
    }

    /// Parse equipped character from equip text.
    /// Text format: "已装备: CharacterName" or similar.
    fn parse_equip_location(&self, text: &str) -> String {
        // "已装备" = "Equipped"
        if text.contains("\u{5DF2}\u{88C5}\u{5907}") {
            let char_name = text
                .replace("\u{5DF2}\u{88C5}\u{5907}", "")
                .replace([':', '\u{FF1A}', ' '], "")
                .trim()
                .to_string();
            if !char_name.is_empty() {
                return fuzzy_match_map(&char_name, &self.mappings.character_name_map)
                    .unwrap_or_default();
            }
        }
        String::new()
    }

    /// Scan all weapons from the backpack.
    ///
    /// Port of `scanAllWeapons()` from GOODScanner/lib/weapon_scanner.js
    pub fn scan(&mut self, skip_open_backpack: bool) -> Result<Vec<GoodWeapon>> {
        info!("[weapon] starting scan...");
        let now = SystemTime::now();

        if !skip_open_backpack {
            navigation::open_backpack(&mut self.system_control, self.config.open_delay);
        }
        navigation::select_backpack_tab(
            "weapon",
            &self.game_info,
            &self.scaler,
            &mut self.system_control,
            self.config.delay_tab,
        );

        // Read item count
        let (_, total_count) = navigation::read_item_count(
            &self.game_info,
            &self.scaler,
            self.capturer.as_ref(),
            self.ocr_model.as_ref(),
        )?;

        if total_count == 0 {
            warn!("[weapon] no weapons in backpack");
            return Ok(Vec::new());
        }
        info!("[weapon] total: {}", total_count);

        let mut weapons: Vec<GoodWeapon> = Vec::new();

        // Inline grid traversal to avoid borrow checker issues with closures
        let total = total_count as usize;
        let items_per_page = GRID_COLS * GRID_ROWS;
        let page_count = (total + items_per_page - 1) / items_per_page;
        let mut item_index = 0;
        let mut scroll_count = 0i32;

        'outer: for page in 0..page_count {
            let mut start_row = 0;
            let remaining = total.saturating_sub(page * items_per_page);

            if remaining < items_per_page {
                let row_count = (remaining + GRID_COLS - 1) / GRID_COLS;
                start_row = GRID_ROWS.saturating_sub(row_count);
                info!(
                    "[weapon] last page: remaining={} rowCount={} startRow={} page={}/{}",
                    remaining, row_count, start_row, page, page_count
                );
            }

            for row in start_row..GRID_ROWS {
                for col in 0..GRID_COLS {
                    if item_index >= total || utils::is_rmb_down() {
                        break 'outer;
                    }

                    navigation::click_grid_item(
                        row, col, &self.game_info, &self.scaler,
                        &mut self.system_control, self.config.delay_grid_item,
                    );
                    utils::sleep((self.config.delay_grid_item / 3).max(1) as u32);

                    let image = match navigation::capture_game_region(
                        &self.game_info, self.capturer.as_ref(),
                    ) {
                        Ok(img) => img,
                        Err(e) => {
                            error!("[weapon] capture failed: {}", e);
                            item_index += 1;
                            continue;
                        }
                    };

                    match self.scan_single_weapon(&image) {
                        Ok(WeaponScanResult::Weapon(weapon)) => {
                            if weapon.rarity >= self.config.min_rarity {
                                if self.config.log_progress {
                                    info!(
                                        "[weapon] {} Lv.{} R{} {}{}",
                                        weapon.key, weapon.level, weapon.refinement,
                                        if weapon.location.is_empty() { "-" } else { &weapon.location },
                                        if weapon.lock { " locked" } else { "" }
                                    );
                                }
                                weapons.push(weapon);
                            }
                        }
                        Ok(WeaponScanResult::Stop) => {
                            break 'outer;
                        }
                        Ok(WeaponScanResult::Skip) => {}
                        Err(e) => {
                            error!("[weapon] scan error: {}", e);
                            if !self.config.continue_on_failure {
                                break 'outer;
                            }
                        }
                    }

                    item_index += 1;
                }
            }

            // Scroll to next page
            if page < page_count - 1 {
                navigation::move_to(
                    &self.game_info, &self.scaler, &mut self.system_control,
                    GRID_FIRST_X, GRID_FIRST_Y,
                );
                utils::sleep(100);
                navigation::scroll_grid_page(
                    &mut self.system_control, &mut scroll_count, self.config.delay_scroll,
                );
            }
        }

        info!(
            "[weapon] complete, {} weapons scanned in {:?}",
            weapons.len(),
            now.elapsed().unwrap_or_default()
        );

        Ok(weapons)
    }
}
