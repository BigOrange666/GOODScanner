use anyhow::Result;
use image::RgbImage;
use log::info;
use regex::Regex;

use yas::capture::Capturer;
use yas::game_info::GameInfo;
use yas::ocr::ImageToText;
use yas::positioning::Rect;
use yas::system_control::SystemControl;
use yas::utils;

use super::constants::*;
use super::coord_scaler::CoordScaler;

/// Navigate to the game's main UI by pressing Escape.
pub fn return_to_main_ui(system_control: &mut SystemControl, delay: u64) {
    system_control.key_press(enigo::Key::Escape).unwrap();
    utils::sleep(delay as u32);
}

/// Open the backpack by pressing B.
///
/// Port of `openBackpack()` from GOODScanner/lib/navigation.js
pub fn open_backpack(
    system_control: &mut SystemControl,
    delay_open: u64,
) {
    system_control.key_press(enigo::Key::Layout('b')).unwrap();
    utils::sleep(delay_open as u32);
}

/// Select a backpack tab by clicking its position.
///
/// Port of `selectBackpackTab()` from GOODScanner/lib/navigation.js
pub fn select_backpack_tab(
    tab: &str,
    game_info: &GameInfo,
    scaler: &CoordScaler,
    system_control: &mut SystemControl,
    delay: u64,
) {
    let (bx, by) = match tab {
        "weapon" => TAB_WEAPON,
        "artifact" => TAB_ARTIFACT,
        _ => {
            log::error!("Unknown backpack tab: {}", tab);
            return;
        }
    };

    click_at(game_info, scaler, system_control, bx, by);
    utils::sleep(delay as u32);
}

/// Open the character screen by pressing C.
///
/// Port of `openCharacterScreen()` from GOODScanner/lib/navigation.js
pub fn open_character_screen(
    system_control: &mut SystemControl,
    delay_open: u64,
) {
    system_control.key_press(enigo::Key::Layout('c')).unwrap();
    utils::sleep((delay_open as f64 * 1.5) as u32);
}

/// Click at a position specified in base 1920x1080 coordinates.
pub fn click_at(
    game_info: &GameInfo,
    scaler: &CoordScaler,
    system_control: &mut SystemControl,
    base_x: f64,
    base_y: f64,
) {
    let origin = game_info.window;
    let x = origin.left as f64 + scaler.scale_x(base_x);
    let y = origin.top as f64 + scaler.scale_y(base_y);
    system_control.mouse_move_to(x as i32, y as i32).unwrap();
    utils::sleep(20);
    system_control.mouse_click().unwrap();
}

/// Move mouse to a position specified in base 1920x1080 coordinates.
pub fn move_to(
    game_info: &GameInfo,
    scaler: &CoordScaler,
    system_control: &mut SystemControl,
    base_x: f64,
    base_y: f64,
) {
    let origin = game_info.window;
    let x = origin.left as f64 + scaler.scale_x(base_x);
    let y = origin.top as f64 + scaler.scale_y(base_y);
    system_control.mouse_move_to(x as i32, y as i32).unwrap();
}

/// Capture the full game window as an RgbImage.
pub fn capture_game_region(
    game_info: &GameInfo,
    capturer: &dyn Capturer<RgbImage>,
) -> Result<RgbImage> {
    capturer.capture_rect(game_info.window)
}

/// Capture a sub-region of the game window.
/// Coordinates are in base 1920x1080 and will be scaled.
pub fn capture_region(
    game_info: &GameInfo,
    scaler: &CoordScaler,
    capturer: &dyn Capturer<RgbImage>,
    base_x: f64,
    base_y: f64,
    base_w: f64,
    base_h: f64,
) -> Result<RgbImage> {
    let rect = Rect {
        left: scaler.scale_x(base_x) as i32,
        top: scaler.scale_y(base_y) as i32,
        width: scaler.scale_x(base_w) as i32,
        height: scaler.scale_y(base_h) as i32,
    };
    capturer.capture_relative_to(rect, game_info.window.origin())
}

/// OCR a region and return trimmed text.
/// Coordinates are in base 1920x1080 and will be scaled.
pub fn ocr_region(
    game_info: &GameInfo,
    scaler: &CoordScaler,
    capturer: &dyn Capturer<RgbImage>,
    ocr_model: &dyn ImageToText<RgbImage>,
    rect: (f64, f64, f64, f64),
) -> Result<String> {
    let (x, y, w, h) = rect;
    let im = capture_region(game_info, scaler, capturer, x, y, w, h)?;
    let text = ocr_model.image_to_text(&im, false)?;
    Ok(text.trim().to_string())
}

/// OCR a region with Y-offset support (for elixir artifacts).
pub fn ocr_region_with_shift(
    game_info: &GameInfo,
    scaler: &CoordScaler,
    capturer: &dyn Capturer<RgbImage>,
    ocr_model: &dyn ImageToText<RgbImage>,
    rect: (f64, f64, f64, f64),
    y_shift: f64,
) -> Result<String> {
    let (x, y, w, h) = rect;
    ocr_region(game_info, scaler, capturer, ocr_model, (x, y + y_shift, w, h))
}

/// Click a grid item at (row, col) in the backpack grid.
///
/// Port of `clickGridItem()` from GOODScanner/lib/navigation.js
pub fn click_grid_item(
    row: usize,
    col: usize,
    game_info: &GameInfo,
    scaler: &CoordScaler,
    system_control: &mut SystemControl,
    delay: u64,
) {
    let item_delay = std::cmp::max(delay / 3, 1);

    let x = GRID_FIRST_X + col as f64 * GRID_OFFSET_X;
    let y = GRID_FIRST_Y + row as f64 * GRID_OFFSET_Y;

    move_to(game_info, scaler, system_control, x, y);
    utils::sleep(item_delay as u32);
    click_at(game_info, scaler, system_control, x, y);
    utils::sleep(item_delay as u32);
}

/// Scroll one grid page (49 wheel ticks, with correction every 3 pages).
///
/// Port of `scrollGridPage()` from GOODScanner/lib/navigation.js
pub fn scroll_grid_page(
    system_control: &mut SystemControl,
    scroll_count: &mut i32,
    delay_scroll: u64,
) {
    for _ in 0..SCROLL_TICKS_PER_PAGE {
        system_control.mouse_scroll(-1, false).unwrap();
    }
    *scroll_count += 1;
    if *scroll_count % SCROLL_CORRECTION_INTERVAL == 0 {
        system_control.mouse_scroll(1, false).unwrap();
    }
    utils::sleep(delay_scroll as u32);
}

/// Read the item count from the backpack header ("X/Y" format).
///
/// Port of `readItemCount()` from GOODScanner/lib/ocr_utils.js
pub fn read_item_count(
    game_info: &GameInfo,
    scaler: &CoordScaler,
    capturer: &dyn Capturer<RgbImage>,
    ocr_model: &dyn ImageToText<RgbImage>,
) -> Result<(i32, i32)> {
    let text = ocr_region(game_info, scaler, capturer, ocr_model, ITEM_COUNT_RECT)?;
    let re = Regex::new(r"(\d+)\s*/\s*(\d+)")?;
    if let Some(caps) = re.captures(&text) {
        let current: i32 = caps[1].parse().unwrap_or(0);
        let total: i32 = caps[2].parse().unwrap_or(0);
        Ok((current, total))
    } else {
        Ok((0, 0))
    }
}

/// Traverse the backpack grid, calling `callback` for each item.
///
/// `callback(item_index)` should return `true` to stop scanning.
/// `on_scroll()` is called after each page scroll.
///
/// Port of `traverseBackpackGrid()` from GOODScanner/lib/navigation.js
pub fn traverse_backpack_grid<F, G>(
    total_count: usize,
    game_info: &GameInfo,
    scaler: &CoordScaler,
    system_control: &mut SystemControl,
    delay_grid_item: u64,
    delay_scroll: u64,
    mut callback: F,
    mut on_scroll: G,
) where
    F: FnMut(usize) -> bool,
    G: FnMut(),
{
    let items_per_page = GRID_COLS * GRID_ROWS;
    let page_count = (total_count + items_per_page - 1) / items_per_page;
    let mut item_index = 0;
    let mut scroll_count = 0i32;

    for page in 0..page_count {
        let mut start_row = 0;
        let remaining = total_count.saturating_sub(page * items_per_page);

        // On the last page, items may not fill all rows
        if remaining < items_per_page {
            let row_count = (remaining + GRID_COLS - 1) / GRID_COLS;
            start_row = GRID_ROWS.saturating_sub(row_count);
            info!(
                "[navigation] last page: remaining={} rowCount={} startRow={} page={}/{}",
                remaining, row_count, start_row, page, page_count
            );
        }

        for row in start_row..GRID_ROWS {
            for col in 0..GRID_COLS {
                if item_index >= total_count {
                    return;
                }

                click_grid_item(row, col, game_info, scaler, system_control, delay_grid_item);
                utils::sleep((delay_grid_item / 3).max(1) as u32);

                if callback(item_index) {
                    return;
                }
                item_index += 1;
            }
        }

        // Scroll to next page (unless this is the last page)
        if page < page_count - 1 {
            move_to(game_info, scaler, system_control, GRID_FIRST_X, GRID_FIRST_Y);
            utils::sleep(100);
            scroll_grid_page(system_control, &mut scroll_count, delay_scroll);
            on_scroll();
        }
    }
}

/// Parse a number from OCR text. Returns the first integer found.
///
/// Port of `parseNumberFromText()` from GOODScanner/lib/ocr_utils.js
pub fn parse_number_from_text(text: &str) -> i32 {
    let re = Regex::new(r"(\d+)").unwrap();
    re.captures(text)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0)
}

/// Parse "XX/YY" format and return the first number.
/// Falls back to extracting any number if the slash format isn't found.
///
/// Port of `parseSlashNumber()` from GOODScanner/lib/ocr_utils.js
pub fn parse_slash_number(text: &str) -> i32 {
    let re = Regex::new(r"(\d+)\s*/\s*(\d+)").unwrap();
    if let Some(caps) = re.captures(text) {
        caps[1].parse().unwrap_or(0)
    } else {
        parse_number_from_text(text)
    }
}

/// Parse "XX/YY" format and return both numbers.
/// Returns (current, max) or (0, 0) if parsing fails.
pub fn parse_slash_pair(text: &str) -> (i32, i32) {
    let re = Regex::new(r"(\d+)\s*/\s*(\d+)").unwrap();
    if let Some(caps) = re.captures(text) {
        let current: i32 = caps[1].parse().unwrap_or(0);
        let max: i32 = caps[2].parse().unwrap_or(0);
        (current, max)
    } else {
        (0, 0)
    }
}
