//! Placeholder functions for in-game UI interactions.
//!
//! These functions require in-game UI coordinate calibration and cannot be
//! implemented without access to a running game client at 1920x1080 resolution.
//!
//! # For the implementing agent / 实现者须知
//!
//! ## Codebase context you need to know
//!
//! **Coordinate system**: All coordinates are at 1920x1080 base resolution.
//! `GenshinGameController` automatically scales them to the actual game resolution
//! via `CoordScaler`. You never need to worry about resolution — just use 1920x1080
//! pixel coordinates from your screenshots.
//!
//! **Key controller methods** (see `scanner/common/game_controller.rs`):
//! - `ctrl.click_at(base_x, base_y)` — moves mouse + 20ms delay + clicks
//! - `ctrl.move_to(base_x, base_y)` — moves mouse without clicking
//! - `ctrl.key_press(enigo::Key::Layout('c'))` — press a keyboard key
//! - `ctrl.key_press(enigo::Key::Escape)` — press Escape
//! - `ctrl.mouse_scroll(amount)` — scroll (positive = down)
//! - `ctrl.capture_game()` — capture full game window as RgbImage
//! - `ctrl.capture_region(x, y, w, h)` — capture sub-region
//! - `ctrl.ocr_region(model, rect)` — capture + OCR in one call
//! - `ctrl.wait_until_panel_loaded(pool_rect, timeout_ms)` — reactive wait
//!   for a panel transition by monitoring red-channel pixel sum changes in a region
//! - `ctrl.return_to_main_ui(max_attempts)` — press Escape up to N times,
//!   verifies main world via Paimon icon brightness
//! - `ctrl.focus_game_window()` — bring game to foreground
//!
//! **Pixel detection helpers** (see `scanner/common/pixel_utils.rs`):
//! - `pixel_utils::detect_artifact_lock(image, scaler, y_shift)` — checks
//!   lock pixels at (1683,428) and (1708,428) with y_shift offset
//! - `pixel_utils::detect_artifact_rarity(image, scaler)` — star color detection
//!
//! **Sleep** (see `yas::utils::sleep(ms)`):
//! - Use for fixed delays between UI actions
//!
//! **Abort check** (`yas::utils::is_rmb_down()`):
//! - Check between long operations; return early if user right-clicked
//!
//! **Existing UI constants** (see `scanner/common/constants.rs`):
//! - `ARTIFACT_LOCK_POS1 = (1683.0, 428.0)` — lock icon pixel position
//! - `ARTIFACT_LOCK_POS2 = (1708.0, 428.0)` — second lock check pixel
//! - `GRID_FIRST_X/Y, GRID_OFFSET_X/Y` — backpack grid layout
//! - `CHAR_NEXT_POS` — "next character" button on character screen
//!
//! ## Implementation order (recommended)
//!
//! 1. `click_lock_button` — easiest, just one click + verify. Required for
//!    lock toggling to work at all. After implementing this, you also need to
//!    complete `lock_manager.rs` Pass 2 (see detailed instructions below).
//!
//! 2. `click_equipment_slot` — 5 fixed coordinates, straightforward.
//!
//! 3. `click_unequip_button` — single button position.
//!
//! 4. `open_character_screen` — medium complexity, requires understanding the
//!    character roster UI.
//!
//! 5. `find_and_click_artifact_in_selection` — hardest, requires understanding
//!    the equip-mode artifact selection list grid.
//!
//! 6. `verify_artifact_equipped` — optional verification, can be deferred.
//!
//! ## How to test
//!
//! Run the server: `GOODScanner.exe --server --port 8765`
//!
//! Send a test lock instruction via curl:
//! ```sh
//! curl -X POST http://127.0.0.1:8765/manage -H "Content-Type: application/json" -d '{
//!   "instructions": [{
//!     "id": "test-lock-1",
//!     "target": {
//!       "setKey": "GladiatorsFinale",
//!       "slotKey": "flower",
//!       "rarity": 5,
//!       "level": 20,
//!       "mainStatKey": "hp",
//!       "substats": [
//!         {"key": "critRate_", "value": 3.9},
//!         {"key": "critDMG_", "value": 7.8}
//!       ]
//!     },
//!     "changes": {"lock": true}
//!   }]
//! }'
//! ```
//!
//! ## lock_manager.rs Pass 2 — completion instructions
//!
//! The lock manager uses a two-pass architecture because `BackpackScanner`
//! borrows `ctrl` mutably during `scan_grid`, preventing us from clicking
//! the lock button inside the scan callback.
//!
//! **Pass 1** (already implemented): scans all artifacts, identifies matches,
//! and records `PendingToggle` entries with `grid_index` and `y_shift`.
//!
//! **Pass 2** (needs completion): after `scan_grid` finishes and releases `ctrl`,
//! we need to navigate back to each matched artifact and click lock. The current
//! code just calls `click_lock_button` without navigating first — this needs to be
//! replaced with:
//!
//! ```text
//! For each PendingToggle (sorted by grid_index ascending for efficiency):
//!   1. If backpack is closed, reopen it (press B, wait, select artifact tab)
//!   2. Calculate which page the grid_index is on:
//!      - page = grid_index / (GRID_COLS * GRID_ROWS)   // 8 * 5 = 40 items per page
//!      - row_in_page = (grid_index % 40) / GRID_COLS
//!      - col = grid_index % GRID_COLS
//!   3. Scroll to the correct page using BackpackScanner::scroll_rows
//!      (or create a new BackpackScanner just for navigation — it's lightweight)
//!   4. Click at the grid position:
//!      - x = GRID_FIRST_X + col * GRID_OFFSET_X
//!      - y = GRID_FIRST_Y + row_in_page * GRID_OFFSET_Y
//!   5. Wait for panel to load (ctrl.wait_until_panel_loaded)
//!   6. Call click_lock_button(ctrl, toggle.y_shift)
//!   7. Wait ~200ms for lock animation
//!   8. Re-capture and verify: pixel_utils::detect_artifact_lock(image, scaler, y_shift)
//!   9. Record success or failure result
//! ```
//!
//! **Optimization**: if toggles are sorted by grid_index, you can scroll forward
//! sequentially rather than jumping to absolute positions.
//!
//! **Alternative simpler approach**: instead of navigating back to grid positions,
//! you could re-scan the backpack in Pass 2 but only process items at known
//! grid_index positions. This avoids the absolute scroll calculation.

use anyhow::{bail, Result};
use image::RgbImage;

use yas::ocr::ImageToText;

use crate::scanner::common::coord_scaler::CoordScaler;
use crate::scanner::common::game_controller::GenshinGameController;
use crate::scanner::common::mappings::MappingManager;

use super::models::ArtifactTarget;

/// Click the lock/unlock button on the artifact detail panel.
///
/// # Difficulty: Easy
///
/// ## Context
///
/// The lock icon is displayed in the artifact detail panel on the right side.
/// Existing code detects lock state via pixels at `ARTIFACT_LOCK_POS1 = (1683, 428)`
/// and `ARTIFACT_LOCK_POS2 = (1708, 428)` — these are detection positions, not
/// necessarily the clickable center.
///
/// When an elixir artifact is displayed, a purple banner pushes all content
/// down by `y_shift` pixels (40.0). The lock icon also shifts down by this amount.
///
/// ## Calibration steps
/// 1. Open artifact backpack at 1920x1080, click on a **locked** artifact
/// 2. Screenshot — note the lock icon's **center** coordinates
/// 3. Click at those coordinates and observe if the lock toggles
///    - If it doesn't toggle, try nearby positions (the clickable area may be
///      larger/different from the icon)
/// 4. Try on an **unlocked** artifact to confirm both directions work
/// 5. Try on an **elixir** artifact (purple banner) — the icon should be at
///    (same_x, same_y + 40)
///
/// ## Implementation
///
/// ```rust
/// pub fn click_lock_button(ctrl: &mut GenshinGameController, y_shift: f64) -> Result<()> {
///     // Replace LOCK_BUTTON_X, LOCK_BUTTON_Y with calibrated coordinates
///     const LOCK_BUTTON_X: f64 = 1683.0; // approximate — calibrate!
///     const LOCK_BUTTON_Y: f64 = 428.0;  // approximate — calibrate!
///     ctrl.click_at(LOCK_BUTTON_X, LOCK_BUTTON_Y + y_shift);
///     yas::utils::sleep(300); // wait for animation
///     Ok(())
/// }
/// ```
pub fn click_lock_button(ctrl: &mut GenshinGameController, _y_shift: f64) -> Result<()> {
    let _ = ctrl;
    bail!(
        "占位符：锁定按钮点击尚未实现。请参阅 ui_actions.rs 中的校准说明。/ \
         Placeholder: lock button click not yet implemented. \
         See ui_actions.rs for calibration instructions."
    )
}

/// Open a specific character's detail screen from the main world.
///
/// # Difficulty: Medium
///
/// ## Context
///
/// The existing character scanner opens the roster by pressing 'C':
/// ```rust
/// ctrl.key_press(enigo::Key::Layout('c'));
/// yas::utils::sleep(1500);
/// ```
/// Then it uses `CHAR_NEXT_POS` to cycle through characters with a "next" button.
///
/// `char_key` is a GOOD format key like "Furina" or "Nahida". The `mappings`
/// parameter has `character_name_map` which maps Chinese names to GOOD keys.
/// You can reverse-lookup: find the Chinese name for the given GOOD key, then
/// OCR character names in the roster to find the right one.
///
/// ## Approach A — sequential cycling (simpler but slower)
/// 1. Press C to open roster
/// 2. Read the current character name (OCR at character name position)
/// 3. If it matches `char_key`, done
/// 4. If not, click `CHAR_NEXT_POS` to go to next character, repeat
/// 5. After cycling through all characters, give up (UiError)
///
/// **Existing constants** (from `scanner/common/constants.rs`):
/// - `CHAR_NEXT_POS = (1845.0, 525.0)` — "next character" button
/// - Character name OCR region: approximately (128, 18, 330, 60)
///
/// ## Approach B — roster list with direct click (faster)
/// 1. Press C to open roster
/// 2. The left sidebar shows character avatars in a scrollable list
/// 3. OCR or icon-match to find the target character
/// 4. Click the character's avatar/entry
/// 5. This may require scrolling the sidebar
///
/// ## Implementation skeleton
///
/// ```rust
/// pub fn open_character_screen(
///     ctrl: &mut GenshinGameController,
///     char_key: &str,
///     mappings: &MappingManager,
/// ) -> Result<()> {
///     // Reverse-lookup: GOOD key -> Chinese name
///     let cn_name = mappings.character_name_map.iter()
///         .find(|(_, v)| v.as_str() == char_key)
///         .map(|(k, _)| k.clone());
///
///     // Press C to open roster
///     ctrl.key_press(enigo::Key::Layout('c'));
///     yas::utils::sleep(1500);
///
///     // Cycle through characters (Approach A)
///     let max_chars = 80; // safety limit
///     for _ in 0..max_chars {
///         // OCR character name
///         // Compare against char_key / cn_name
///         // If match: return Ok(())
///         // If not: click next
///         ctrl.click_at(1845.0, 525.0); // CHAR_NEXT_POS
///         yas::utils::sleep(300);
///     }
///     bail!("Character not found")
/// }
/// ```
pub fn open_character_screen(
    ctrl: &mut GenshinGameController,
    _char_key: &str,
    _mappings: &MappingManager,
) -> Result<()> {
    let _ = ctrl;
    bail!(
        "占位符：角色界面导航尚未实现。/ \
         Placeholder: character screen navigation not yet implemented."
    )
}

/// Click an artifact equipment slot on the character detail screen.
///
/// # Difficulty: Easy
///
/// ## Context
///
/// On the character detail screen, the 5 artifact slots are displayed in a
/// fixed layout (usually on the right side). Each slot shows the equipped
/// artifact's icon or an empty slot placeholder.
///
/// The slot order follows the GOOD convention:
/// - "flower" (生之花) — typically top or first
/// - "plume" (死之羽)
/// - "sands" (时之沙)
/// - "goblet" (空之杯)
/// - "circlet" (理之冠) — typically bottom or last
///
/// ## Calibration steps
/// 1. Open any character's detail screen at 1920x1080
/// 2. Screenshot — identify the center of each of the 5 artifact slot icons
/// 3. Note down coordinates for each slot
///
/// ## Implementation
///
/// ```rust
/// pub fn click_equipment_slot(
///     ctrl: &mut GenshinGameController,
///     slot_key: &str,
/// ) -> Result<()> {
///     // Replace with calibrated coordinates
///     let (x, y) = match slot_key {
///         "flower"  => (TODO_X, TODO_Y),
///         "plume"   => (TODO_X, TODO_Y),
///         "sands"   => (TODO_X, TODO_Y),
///         "goblet"  => (TODO_X, TODO_Y),
///         "circlet" => (TODO_X, TODO_Y),
///         _ => bail!("Unknown slot: {}", slot_key),
///     };
///     ctrl.click_at(x, y);
///     yas::utils::sleep(500); // wait for selection list to open
///     Ok(())
/// }
/// ```
pub fn click_equipment_slot(
    ctrl: &mut GenshinGameController,
    _slot_key: &str,
) -> Result<()> {
    let _ = ctrl;
    bail!(
        "占位符：装备栏位点击尚未实现。/ \
         Placeholder: equipment slot click not yet implemented."
    )
}

/// Click the "unequip" button to remove an artifact from the current slot.
///
/// # Difficulty: Easy
///
/// ## Context
///
/// When viewing an equipped artifact's detail in the character screen,
/// there should be a "取下" (unequip) button, typically at the bottom
/// of the artifact detail panel.
///
/// ## Calibration steps
/// 1. Open character detail, click an equipped artifact slot
/// 2. The artifact detail panel should show with a "取下" button
/// 3. Screenshot — find the button center coordinates
///
/// ## Implementation
///
/// ```rust
/// pub fn click_unequip_button(ctrl: &mut GenshinGameController) -> Result<()> {
///     ctrl.click_at(UNEQUIP_BUTTON_X, UNEQUIP_BUTTON_Y);
///     yas::utils::sleep(500);
///     Ok(())
/// }
/// ```
pub fn click_unequip_button(ctrl: &mut GenshinGameController) -> Result<()> {
    let _ = ctrl;
    bail!(
        "占位符：卸下按钮点击尚未实现。/ \
         Placeholder: unequip button click not yet implemented."
    )
}

/// Find and click a target artifact in the artifact selection list.
///
/// # Difficulty: Hard
///
/// ## Context
///
/// When a character's artifact slot is clicked, the game shows a scrollable
/// list/grid of available artifacts for that slot type. This is different from
/// the main backpack — it only shows artifacts of the matching slot type and
/// may have a different grid layout.
///
/// ## What you need to discover via screenshots
///
/// 1. **Grid layout**: how many rows/cols? What are the first item coordinates
///    and spacing? Compare with the main backpack grid:
///    - Main backpack: 8 cols × 5 rows, first at (180, 253), offset (145, 166)
///    - The selection list grid may be different!
///
/// 2. **Detail panel**: when you click an item in the selection list, does a
///    detail panel appear (like the main backpack)? If so, you can reuse the
///    existing `GoodArtifactScanner::identify_artifact()` function on that panel.
///
/// 3. **Scroll behavior**: how many ticks per page? Is there a scroll bar?
///    Compare with main backpack's 49 ticks per 5-row page.
///
/// 4. **Equip action**: after clicking an artifact, does the game:
///    - Equip immediately? (most likely)
///    - Show a confirmation dialog?
///    - Show an "already equipped on X, swap?" dialog?
///
/// ## Implementation approach
///
/// The most reliable approach mirrors how the existing backpack scanner works:
///
/// ```rust
/// pub fn find_and_click_artifact_in_selection(
///     ctrl: &mut GenshinGameController,
///     target: &ArtifactTarget,
///     ocr: &dyn ImageToText<RgbImage>,
///     scaler: &CoordScaler,
///     mappings: &MappingManager,
/// ) -> Result<bool> {
///     // You'll need a second OCR model for substats (v4), similar to:
///     // let substat_ocr = ocr_factory::create_ocr_model("ppocrv4")?;
///
///     // Iterate visible grid positions:
///     for row in 0..SELECTION_GRID_ROWS {
///         for col in 0..SELECTION_GRID_COLS {
///             let x = SELECTION_FIRST_X + col as f64 * SELECTION_OFFSET_X;
///             let y = SELECTION_FIRST_Y + row as f64 * SELECTION_OFFSET_Y;
///
///             // Click item to show detail panel
///             ctrl.click_at(x, y);
///             ctrl.wait_until_panel_loaded(PANEL_POOL_RECT, 400);
///             yas::utils::sleep(100);
///
///             // Capture and identify
///             let image = ctrl.capture_game()?;
///             if let Ok(Some(artifact)) = GoodArtifactScanner::identify_artifact(
///                 ocr, substat_ocr, &image, scaler, mappings,
///             ) {
///                 // Check if this matches our target
///                 if matching::match_score(&artifact, target).is_some() {
///                     // Found it! Click to equip (or it may already be selected)
///                     // May need to click an "equip" button
///                     return Ok(true);
///                 }
///             }
///         }
///     }
///
///     // Scroll and repeat until no more items
///     // ...
///
///     Ok(false)
/// }
/// ```
///
/// **Performance note**: this is O(n) where n = number of artifacts of this slot type.
/// For a typical account with ~300 artifacts, each slot type has ~60 artifacts.
/// At ~200ms per item (click + OCR), that's ~12 seconds per slot — acceptable.
///
/// **Alternative approach**: if the selection list shows enough info per item
/// (set icon, level, rarity) without clicking, you could do a faster visual scan.
/// But OCR on the detail panel is more reliable and reuses existing code.
pub fn find_and_click_artifact_in_selection(
    ctrl: &mut GenshinGameController,
    _target: &ArtifactTarget,
    _ocr: &dyn ImageToText<RgbImage>,
    _scaler: &CoordScaler,
    _mappings: &MappingManager,
) -> Result<bool> {
    let _ = ctrl;
    bail!(
        "占位符：圣遗物选择列表扫描尚未实现。/ \
         Placeholder: artifact selection list scanning not yet implemented."
    )
}

/// Verify that the expected artifact is now equipped in the given slot.
///
/// # Difficulty: Medium (optional — can be deferred)
///
/// ## Context
///
/// After equipping, the character screen should update to show the new artifact
/// in the slot. This function verifies the change actually took effect.
///
/// You could skip this initially and just trust the click succeeded. Add
/// verification later if you observe reliability issues.
///
/// ## Possible approaches
///
/// 1. **OCR approach**: OCR the artifact name/set visible on the character screen
///    and compare against `target.set_key` + `target.slot_key`
///
/// 2. **Pixel approach**: check if the slot icon changed (compare before/after
///    screenshots at the slot position)
///
/// 3. **Re-click approach**: click the slot again to open its detail panel,
///    then use `identify_artifact` to verify — most reliable but slow
pub fn verify_artifact_equipped(
    ctrl: &GenshinGameController,
    _slot_key: &str,
    _target: &ArtifactTarget,
    _scaler: &CoordScaler,
) -> Result<bool> {
    let _ = ctrl;
    bail!(
        "占位符：装备验证尚未实现。/ \
         Placeholder: equip verification not yet implemented."
    )
}

/// Leave the character screen and return to main world.
///
/// Already implemented — delegates to `ctrl.return_to_main_ui()`.
pub fn leave_character_screen(ctrl: &mut GenshinGameController) -> Result<()> {
    ctrl.return_to_main_ui(4);
    Ok(())
}
