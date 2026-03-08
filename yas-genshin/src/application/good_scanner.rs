use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use clap::{command, ArgMatches, Args, FromArgMatches};
use log::info;

use yas::game_info::{GameInfo, GameInfoBuilder};

use crate::scanner::good_artifact_scanner::{GoodArtifactScanner, GoodArtifactScannerConfig};
use crate::scanner::good_character_scanner::{GoodCharacterScanner, GoodCharacterScannerConfig};
use crate::scanner::good_common::mappings::{MappingManager, NameOverrides};
use crate::scanner::good_common::models::GoodExport;
use crate::scanner::good_weapon_scanner::{GoodWeaponScanner, GoodWeaponScannerConfig};

#[derive(Clone, clap::Args)]
pub struct GoodScannerConfig {
    /// Scan characters
    #[arg(long = "good-scan-characters", help = "Scan characters")]
    pub scan_characters: bool,

    /// Scan weapons
    #[arg(long = "good-scan-weapons", help = "Scan weapons")]
    pub scan_weapons: bool,

    /// Scan artifacts
    #[arg(long = "good-scan-artifacts", help = "Scan artifacts")]
    pub scan_artifacts: bool,

    /// Scan everything
    #[arg(long = "good-scan-all", help = "Scan all (characters + weapons + artifacts)")]
    pub scan_all: bool,

    /// Output directory
    #[arg(long = "good-output-dir", help = "Output directory", default_value = ".")]
    pub output_dir: String,

    /// Custom Traveler name (if renamed in-game)
    #[arg(long = "good-traveler-name", help = "Custom Traveler name")]
    pub traveler_name: Option<String>,

    /// Custom Wanderer name
    #[arg(long = "good-wanderer-name", help = "Custom Wanderer name")]
    pub wanderer_name: Option<String>,

    /// Custom Manekin name
    #[arg(long = "good-manekin-name", help = "Custom Manekin name")]
    pub manekin_name: Option<String>,

    /// Custom Manekina name
    #[arg(long = "good-manekina-name", help = "Custom Manekina name")]
    pub manekina_name: Option<String>,
}

pub struct GoodScannerApplication {
    arg_matches: ArgMatches,
}

impl GoodScannerApplication {
    pub fn new(matches: ArgMatches) -> Self {
        Self {
            arg_matches: matches,
        }
    }

    pub fn build_command() -> clap::Command {
        let mut cmd = command!();
        cmd = <GoodScannerConfig as Args>::augment_args_for_update(cmd);
        cmd = <GoodCharacterScannerConfig as Args>::augment_args_for_update(cmd);
        cmd = <GoodWeaponScannerConfig as Args>::augment_args_for_update(cmd);
        cmd = <GoodArtifactScannerConfig as Args>::augment_args_for_update(cmd);
        cmd
    }

    fn get_game_info() -> Result<GameInfo> {
        GameInfoBuilder::new()
            .add_local_window_name("\u{539F}\u{795E}") // 原神
            .add_local_window_name("Genshin Impact")
            .add_cloud_window_name("\u{4E91}\u{00B7}\u{539F}\u{795E}") // 云·原神
            .build()
    }

    pub fn run(&self) -> Result<()> {
        let arg_matches = &self.arg_matches;
        let config = GoodScannerConfig::from_arg_matches(arg_matches)?;
        let game_info = Self::get_game_info()?;

        info!("window: {:?}", game_info.window);
        info!("ui: {:?}", game_info.ui);
        info!("cloud: {}", game_info.is_cloud);

        #[cfg(target_os = "windows")]
        {
            if !yas::utils::is_admin() {
                return Err(anyhow!("Please run as administrator"));
            }
        }

        // Determine what to scan
        let scan_characters = config.scan_characters || config.scan_all;
        let scan_weapons = config.scan_weapons || config.scan_all;
        let scan_artifacts = config.scan_artifacts || config.scan_all
            || (!config.scan_characters && !config.scan_weapons);

        // Fetch and load mappings
        info!("=== Loading mappings ===");
        let overrides = NameOverrides {
            traveler_name: config.traveler_name.clone(),
            wanderer_name: config.wanderer_name.clone(),
            manekin_name: config.manekin_name.clone(),
            manekina_name: config.manekina_name.clone(),
        };
        let mappings = Rc::new(MappingManager::new(&overrides)?);
        info!(
            "Loaded {} characters, {} weapons, {} artifact sets",
            mappings.character_name_map.len(),
            mappings.weapon_name_map.len(),
            mappings.artifact_set_map.len(),
        );

        let mut characters = None;
        let mut weapons = None;
        let mut artifacts = None;

        // Scan characters
        if scan_characters {
            info!("=== Scanning characters ===");
            let char_config = GoodCharacterScannerConfig::from_arg_matches(arg_matches)?;
            let mut scanner = GoodCharacterScanner::new(
                char_config,
                game_info.clone(),
                mappings.clone(),
            )?;
            let result = scanner.scan()?;
            info!("Scanned {} characters", result.len());
            characters = Some(result);

            // Return to main UI before next scan
            let mut system_control = yas::system_control::SystemControl::new();
            system_control.key_press(enigo::Key::Escape).unwrap();
            yas::utils::sleep(1000);
        }

        // Scan weapons
        if scan_weapons {
            info!("=== Scanning weapons ===");
            let weapon_config = GoodWeaponScannerConfig::from_arg_matches(arg_matches)?;
            let mut scanner = GoodWeaponScanner::new(
                weapon_config,
                game_info.clone(),
                mappings.clone(),
            )?;
            let result = scanner.scan(false)?;
            info!("Scanned {} weapons", result.len());
            weapons = Some(result);
        }

        // Scan artifacts
        if scan_artifacts {
            info!("=== Scanning artifacts ===");
            let artifact_config = GoodArtifactScannerConfig::from_arg_matches(arg_matches)?;
            // If weapons were just scanned, we're already in the backpack
            let skip_open = scan_weapons;
            let mut scanner = GoodArtifactScanner::new(
                artifact_config,
                game_info.clone(),
                mappings.clone(),
            )?;
            let result = scanner.scan(skip_open)?;
            info!("Scanned {} artifacts", result.len());
            artifacts = Some(result);
        }

        // Export as GOOD v3
        let export = GoodExport::new(characters, weapons, artifacts);
        let json = serde_json::to_string_pretty(&export)?;

        let timestamp = chrono_timestamp();
        let output_dir = PathBuf::from(&config.output_dir);
        std::fs::create_dir_all(&output_dir)?;
        let filename = format!("good_export_{}.json", timestamp);
        let path = output_dir.join(&filename);

        std::fs::write(&path, &json)?;
        info!("Exported to {}", path.display());

        Ok(())
    }
}

/// Generate a timestamp string like "2024-01-15_12-30-45"
fn chrono_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Simple timestamp without chrono dependency
    let secs_per_day = 86400u64;
    let secs_per_hour = 3600u64;
    let secs_per_min = 60u64;

    // Days since epoch
    let days = now / secs_per_day;
    let remaining = now % secs_per_day;
    let hours = remaining / secs_per_hour;
    let remaining = remaining % secs_per_hour;
    let minutes = remaining / secs_per_min;
    let seconds = remaining % secs_per_min;

    // Calculate year/month/day from days since epoch (simplified)
    let mut y = 1970i32;
    let mut d = days as i32;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }

    let months_days: &[i32] = if is_leap(y) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 1;
    for &md in months_days {
        if d < md {
            break;
        }
        d -= md;
        m += 1;
    }

    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        y, m, d + 1, hours, minutes, seconds
    )
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
