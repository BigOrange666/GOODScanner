# Yas — Genshin Impact Scanner

## Overview

Yas (Yet Another Scanner) is a Rust application that scans Genshin Impact in-game data (characters, weapons, artifacts) using OCR and exports it in **GOOD v3** (Genshin Open Object Description) format for use with optimizer tools.

## Architecture

### Workspace Crates

- **`yas`** (`yas_core`) — Platform-agnostic core library: screen capture, OCR (PaddlePaddle ONNX models), system control (mouse/keyboard), game window detection, positioning/scaling utilities.
- **`yas-genshin`** (`yas_scanner_genshin`) — Genshin-specific scanner logic: GOOD v3 scanners for characters, weapons, and artifacts. Handles in-game navigation, panel OCR, and name matching via remote mappings.
- **`yas-application`** — Binary crate. Single target: `yas.exe`.

### Key Modules (yas-genshin)

```
src/
├── application/
│   └── good_scanner.rs       # CLI entry point, orchestrates all scanning
├── scanner/
│   ├── good_common/           # Shared scanner infrastructure
│   │   ├── game_controller.rs # Mouse/keyboard/capture control
│   │   ├── backpack_scanner.rs# Grid-based inventory navigation
│   │   ├── mappings.rs        # Remote name→GOOD key mappings (from ggartifact.com)
│   │   ├── coord_scaler.rs    # Resolution-independent coordinate scaling (base: 1920x1080)
│   │   ├── models.rs          # GOOD v3 data models (GoodExport, GoodCharacter, etc.)
│   │   ├── stat_parser.rs     # Artifact stat string parsing
│   │   ├── diff.rs            # Groundtruth comparison tooling
│   │   ├── constants.rs       # Grid positions, UI coordinates
│   │   ├── ocr_factory.rs     # OCR backend selection (ppocrv3/v4/v5)
│   │   ├── pixel_utils.rs     # Color/pixel analysis helpers
│   │   ├── fuzzy_match.rs     # Fuzzy string matching for OCR results
│   │   └── navigation.rs      # Tab/page navigation helpers
│   ├── good_character_scanner/ # Character panel OCR
│   ├── good_weapon_scanner/    # Weapon panel OCR
│   └── good_artifact_scanner/  # Artifact panel OCR
```

### How Scanning Works

1. User opens Genshin Impact and navigates to the appropriate screen
2. `GenshinGameController` captures the game window and provides scaled coordinates
3. `BackpackScanner` navigates the grid inventory (weapons/artifacts)
4. Individual scanners OCR each panel's fields (name, level, stats, etc.)
5. OCR results are fuzzy-matched against `MappingManager` data (fetched from ggartifact.com)
6. Results are exported as GOOD v3 JSON

### Config File

On first run, `good_config.json` is created next to the exe. Users fill in custom in-game names for Traveler/Wanderer/Manekin/Manekina (renameable characters).

## Build & Run

```bash
# Stable Rust toolchain
rustup default stable

# Build
cargo build --release

# The binary is at target/release/yas.exe
# Run with default (scan artifacts):
yas.exe

# Scan everything:
yas.exe --good-scan-all

# Scan specific categories:
yas.exe --good-scan-characters --good-scan-weapons --good-scan-artifacts
```

Requires administrator privileges on Windows (for input simulation).

## CLI Flags

All flags are prefixed with `--good-*` for the main scanner config, plus per-scanner flags (see `--help`).

Key flags:
- `--good-scan-all` / `--good-scan-characters` / `--good-scan-weapons` / `--good-scan-artifacts`
- `--good-output-dir <DIR>` — output directory (default: `.`)
- `--good-traveler-name` / `--good-wanderer-name` — override config file names
- `--good-ocr-backend <ppocrv3|ppocrv4|ppocrv5>` — OCR model (default: ppocrv5)
- `--good-debug-compare <PATH>` — compare output against groundtruth JSON
- `--good-debug-timing` — show per-field OCR timing

## Dependencies & Platform

- **OCR**: ONNX Runtime (`ort` crate) with PaddleOCR models (embedded via `include_bytes!`)
- **Screen capture**: `screenshots` crate + `windows-capture` on Windows
- **Input simulation**: `enigo` crate
- **Remote mappings**: `reqwest` (blocking HTTP to ggartifact.com)
- **Windows only**: Requires admin, uses Win32 APIs for window detection

## Conventions

- All UI coordinates use 1920x1080 as base resolution, scaled at runtime via `CoordScaler`
- Chinese (zh_CN) game client only — OCR models trained on Chinese game text
- GOOD v3 format spec: keys use PascalCase (e.g., `"SkywardHarp"`, `"Furina"`)
- The `data/` directory (gitignored) caches remote mapping files

## Artifact Scanner Details

### Dual-Engine OCR Pipeline

The artifact scanner uses two OCR backends simultaneously:
- **Main engine** (ppocrv5): Part name, main stat, set name, equip text
- **Substat engine** (ppocrv4): Substats and level (better for small numbers)

Both engines OCR each substat line. Results are collected as `OcrCandidate` lists per line, then validated by the roll solver.

### Roll Solver (`roll_solver.rs`)

Validates substat combinations against game mechanics:
- Uses pre-computed **rollTable** lookup (from `rollTable.json` via `roll_table.rs`) — NOT brute-force f64 enumeration
- Each entry is `(display_value×10: i32, roll_count_bitmask: u8)`, binary searched
- Validates total roll count = init_count + level/4
- **Init preference**: Level 0 → prefer higher init first (lines = init count); Level > 0 → prefer lower init (better accuracy)
- Outputs `totalRolls`, `initialValue` per substat, and `inactive` flag
- The solver treats inactive (待激活) substats identically to active ones — their values are real roll values

### Elixir Crafted Detection

Elixir artifacts display a purple banner ("祝圣之霜定义") that shifts all content down by 40px (`ELIXIR_SHIFT`).
- Detection: 3 pixels at (1510, 1520, 1530), y=423 — checks for purple (blue > 230 && blue > green + 40)
- **Do NOT move to x=1683** — that hits the lock icon and causes massive false positives
- When detected, all subsequent OCR regions are Y-shifted by 40px

### Substat Crop Regions

- Lines 0–2: width 255px (calibrated to avoid OCR noise from wider crops)
- Line 3: width 355px (wider to capture "(待激活)" text on unactivated substats)
- All start at x=1356

### Unactivated Substats (待激活)

- Appear on level-0 artifacts as the 4th substat line with muted font and "(待激活)" appended
- The stat key and value are real (not zero) — it's the value that WILL be added on first level-up
- `stat_parser.rs` detects "(待激活)" text and sets `ParsedStat.inactive = true`, keeping the real value
- `OcrCandidate.inactive` propagates through the solver to `SolvedSubstat.inactive`
- Scanner splits solver results into `substats` (active) and `unactivated_substats` (inactive) in the output

### Pixel-Based Detection (highly reliable)

- **Rarity**: Star pixel color at fixed Y positions
- **Lock**: Pixel color at `ARTIFACT_LOCK_POS1` (1683, 428)
- **Elixir**: Purple banner check at (1510–1530, 423)
- **Astral mark**: Pixel at `ARTIFACT_ASTRAL_POS1`

### Parallelization

- `OcrPool`: Channel-based pool of N OCR model instances
- `scan_worker`: Generic parallel worker for backpack grid items
- **ALWAYS create separate pools** for main and substat OCR (sharing causes deadlock: N tasks each hold 1 instance, all waiting for a 2nd)

## Testing & Validation

### Groundtruth

- `genshin_export.json`: Exported via third-party tool, contains complete artifact/character/weapon data
- Note: GT uses typo `elixerCrafted` (not `elixirCrafted`) — diff report handles both

### Diff Report (`diff_report.py`)

- Compares scan output against groundtruth with Hungarian algorithm matching
- Groups by `(setKey, slotKey, rarity, lock)` — rarity and lock are hard matching requirements (pixel-based, very reliable)
- Three-tier categorization: non-stat diffs, stat-key diffs, stat-value-only diffs
- Always run scans with `--good-dump-images` so dump images match the scan output
- Use `python diff_report.py <scan.json> <gt.json>` to generate `diff_report.md`

### Other Scripts

- `test_solver.py`: Validates roll solver against groundtruth (expects ~99.7% totalRolls accuracy)
- `gen_roll_table.py`: Generates `roll_table.rs` from `rollTable.json`

### Key Calibration Values

| Parameter | Value | Notes |
|-----------|-------|-------|
| Substat width (lines 0–2) | 255px | Wider causes OCR failures |
| Substat width (line 3) | 355px | Captures "(待激活)" text |
| delay_after_panel | 100ms | Lock/astral mark animation |
| Talent overview width | 90px | Supports 2-digit levels |
| ELIXIR_SHIFT | 40px | Purple banner height |
| Elixir pixel positions | (1510–1530, 423) | Do NOT use x=1683 |
