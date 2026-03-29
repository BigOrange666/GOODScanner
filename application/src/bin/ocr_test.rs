//! Quick CLI tool to OCR a given image using ppocrv4 and ppocrv5.
//!
//! Usage:
//!   ocr_test <image_path> [--crop-right <pixels>]
//!   ocr_test --equip <image_path>
//!   ocr_test --char-name <image_path>
//!   ocr_test --eval-ocr <debug_images_dir>
//!   ocr_test --reprocess <images_dir> [--output <json_path>]
//!
//! Default mode: runs both engines, prints raw text and stat parsing results.
//! --equip mode: tests the full equip pipeline (OCR → parse → fuzzy match).
//! --char-name mode: tests character name pipeline (OCR → parse → fuzzy match).
//! --eval-ocr mode: batch evaluate v4 vs v5 accuracy on character names and equip text.
//! --reprocess mode: re-runs artifact scanner on dumped full.png images, outputs GOOD JSON.

use anyhow::Result;
use yas_genshin::scanner::common::equip_parser;
use yas_genshin::scanner::common::fuzzy_match::fuzzy_match_map;
use yas_genshin::scanner::common::ocr_factory::create_ocr_model;
use yas_genshin::scanner::common::mappings::{MappingManager, NameOverrides};

fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: ocr_test <image_path> [--crop-right <pixels>]");
        eprintln!("       ocr_test --equip <image_path>");
        eprintln!("       ocr_test --char-name <image_path>");
        eprintln!("       ocr_test --eval-ocr <debug_images_dir>");
        eprintln!("       ocr_test --reprocess <images_dir> [--output <json_path>]");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "--equip" => {
            if args.len() < 3 {
                eprintln!("Usage: ocr_test --equip <image_path>");
                std::process::exit(1);
            }
            run_equip_test(&args[2])
        }
        "--char-name" => {
            if args.len() < 3 {
                eprintln!("Usage: ocr_test --char-name <image_path>");
                std::process::exit(1);
            }
            run_char_name_test(&args[2])
        }
        "--eval-ocr" => {
            if args.len() < 3 {
                eprintln!("Usage: ocr_test --eval-ocr <debug_images_dir>");
                std::process::exit(1);
            }
            run_eval_ocr(&args[2])
        }
        "--reprocess" => {
            if args.len() < 3 {
                eprintln!("Usage: ocr_test --reprocess <images_dir> [--output <json_path>]");
                std::process::exit(1);
            }
            let output = if args.len() >= 5 && args[3] == "--output" {
                Some(args[4].as_str())
            } else {
                None
            };
            run_reprocess(&args[2], output)
        }
        _ => run_ocr_test(&args),
    }
}

fn run_ocr_test(args: &[String]) -> Result<()> {
    let image_path = &args[1];
    let mut crop_right: u32 = 0;
    if args.len() >= 4 && args[2] == "--crop-right" {
        crop_right = args[3].parse().unwrap_or(0);
    }

    // Load image
    let img = image::open(image_path)?.to_rgb8();
    println!("Image: {} ({}x{})", image_path, img.width(), img.height());

    // Optionally crop right side
    let img = if crop_right > 0 && crop_right < img.width() {
        let new_w = img.width() - crop_right;
        println!("Cropping {}px from right -> {}x{}", crop_right, new_w, img.height());
        image::imageops::crop_imm(&img, 0, 0, new_w, img.height()).to_image()
    } else {
        img
    };

    // Load models
    println!("Loading models...");
    let v4 = create_ocr_model("ppocrv4")?;
    let v5 = create_ocr_model("ppocrv5")?;

    // Run OCR
    let result_v4 = v4.image_to_text(&img, false)?;
    let result_v5 = v5.image_to_text(&img, false)?;

    println!();
    println!("ppocrv4: {:?}", result_v4);
    println!("ppocrv5: {:?}", result_v5);

    // Also try stat parsing
    let parsed_v4 = yas_genshin::scanner::common::stat_parser::parse_stat_from_text(result_v4.trim());
    let parsed_v5 = yas_genshin::scanner::common::stat_parser::parse_stat_from_text(result_v5.trim());

    println!();
    if let Some(p) = &parsed_v4 {
        println!("ppocrv4 parsed: key={}, value={}, inactive={}", p.key, p.value, p.inactive);
    } else {
        println!("ppocrv4 parsed: None");
    }
    if let Some(p) = &parsed_v5 {
        println!("ppocrv5 parsed: key={}, value={}, inactive={}", p.key, p.value, p.inactive);
    } else {
        println!("ppocrv5 parsed: None");
    }

    Ok(())
}

fn run_reprocess(images_dir: &str, output_path: Option<&str>) -> Result<()> {
    use yas_genshin::scanner::artifact::{
        GoodArtifactScanner, ArtifactOcrRegions, ArtifactScanResult, GoodArtifactScannerConfig,
    };
    use yas_genshin::scanner::common::coord_scaler::CoordScaler;
    use yas_genshin::scanner::common::models::GoodExport;

    // Discover NNNN/full.png entries
    let mut entries: Vec<(usize, std::path::PathBuf)> = Vec::new();
    let artifacts_dir = std::path::Path::new(images_dir);
    if !artifacts_dir.is_dir() {
        anyhow::bail!("{} is not a directory", images_dir);
    }
    for entry in std::fs::read_dir(artifacts_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Ok(idx) = name.parse::<usize>() {
            let full_path = entry.path().join("full.png");
            if full_path.exists() {
                entries.push((idx, full_path));
            }
        }
    }
    entries.sort_by_key(|(idx, _)| *idx);
    eprintln!("Found {} artifact images in {}", entries.len(), images_dir);

    // Load models + mappings
    eprintln!("Loading OCR models...");
    let v4 = create_ocr_model("ppocrv4")?;
    let v5 = create_ocr_model("ppocrv5")?;
    eprintln!("Loading mappings...");
    let mappings = MappingManager::new(&NameOverrides::default())?;

    let regions = ArtifactOcrRegions::new();
    let config = GoodArtifactScannerConfig {
        continue_on_failure: true,
        dump_images: false,
        verbose: true,
        min_rarity: 1,
        ..Default::default()
    };

    let mut artifacts = Vec::new();
    let mut errors = 0;

    for (idx, path) in &entries {
        let img = image::open(path)?.to_rgb8();
        let scaler = CoordScaler::new(img.width(), img.height());

        match GoodArtifactScanner::scan_single_artifact(
            &*v5, &*v4, &img, &scaler, &regions, &mappings, &config, *idx,
        ) {
            Ok(ArtifactScanResult::Artifact(artifact)) => {
                artifacts.push(artifact);
            }
            Ok(ArtifactScanResult::Stop) => {
                eprintln!("[{:04}] low rarity, skipped", idx);
            }
            Ok(ArtifactScanResult::Skip) => {
                eprintln!("[{:04}] skipped", idx);
            }
            Err(e) => {
                eprintln!("[{:04}] ERROR: {}", idx, e);
                errors += 1;
            }
        }
    }

    eprintln!("Reprocessed: {} artifacts, {} errors", artifacts.len(), errors);

    let export = GoodExport::new(None, None, Some(artifacts));
    let json = serde_json::to_string_pretty(&export)?;

    if let Some(out) = output_path {
        std::fs::write(out, &json)?;
        eprintln!("Written to {}", out);
    } else {
        println!("{}", json);
    }

    Ok(())
}

/// Parse a character name from OCR text using the same logic as the scanner.
/// Extracts the name part after "/" and fuzzy matches against the character map.
fn parse_char_name(text: &str, char_map: &std::collections::HashMap<String, String>) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let slash_char = if text.contains('/') { Some('/') } else if text.contains('\u{FF0F}') { Some('\u{FF0F}') } else { None };
    if let Some(slash) = slash_char {
        let idx = text.find(slash).unwrap();
        let raw_name: String = text[idx + slash.len_utf8()..]
            .chars()
            .filter(|c| {
                matches!(*c, '\u{4E00}'..='\u{9FFF}' | '\u{300C}' | '\u{300D}' | 'a'..='z' | 'A'..='Z' | '0'..='9')
            })
            .collect();
        fuzzy_match_map(&raw_name, char_map)
    } else {
        fuzzy_match_map(text, char_map)
    }
}

fn run_char_name_test(image_path: &str) -> Result<()> {
    let img = image::open(image_path)?.to_rgb8();
    println!("Image: {} ({}x{})", image_path, img.width(), img.height());

    println!("Loading mappings...");
    let mappings = MappingManager::new(&NameOverrides::default())?;

    println!("Loading models...");
    let v4 = create_ocr_model("ppocrv4")?;
    let v5 = create_ocr_model("ppocrv5")?;

    let text_v4 = v4.image_to_text(&img, false)?;
    let text_v5 = v5.image_to_text(&img, false)?;

    let name_v4 = parse_char_name(&text_v4, &mappings.character_name_map);
    let name_v5 = parse_char_name(&text_v5, &mappings.character_name_map);

    println!();
    println!("v4 OCR:   {:?}", text_v4);
    println!("v4 match: {:?}", name_v4);
    println!();
    println!("v5 OCR:   {:?}", text_v5);
    println!("v5 match: {:?}", name_v5);

    println!();
    println!("=== Combined (v4 → v5 fallback) ===");
    if let Some(ref n) = name_v4 {
        println!("v4 matched: {}", n);
    } else if let Some(ref n) = name_v5 {
        println!("v4 failed, v5 fallback matched: {}", n);
    } else {
        println!("BOTH FAILED");
    }

    Ok(())
}

fn run_eval_ocr(debug_dir: &str) -> Result<()> {
    use std::path::Path;

    let base = Path::new(debug_dir);

    println!("Loading models...");
    let v4 = create_ocr_model("ppocrv4")?;
    let v5 = create_ocr_model("ppocrv5")?;
    println!("Loading mappings...");
    let mappings = MappingManager::new(&NameOverrides::default())?;

    // Evaluate character names
    let char_dir = base.join("characters");
    if char_dir.is_dir() {
        println!();
        println!("=== Character Name OCR (v4 vs v5) ===");
        let mut entries: Vec<(usize, std::path::PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&char_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(idx) = name.parse::<usize>() {
                let path = entry.path().join("name.png");
                if path.exists() {
                    entries.push((idx, path));
                }
            }
        }
        entries.sort_by_key(|(idx, _)| *idx);

        let mut v4_ok = 0;
        let mut v5_ok = 0;
        let mut both_fail = 0;
        let mut disagree = Vec::new();

        for (idx, path) in &entries {
            let img = image::open(path)?.to_rgb8();
            let text_v4 = v4.image_to_text(&img, false)?;
            let text_v5 = v5.image_to_text(&img, false)?;
            let name_v4 = parse_char_name(&text_v4, &mappings.character_name_map);
            let name_v5 = parse_char_name(&text_v5, &mappings.character_name_map);

            let v4_matched = name_v4.is_some();
            let v5_matched = name_v5.is_some();
            if v4_matched { v4_ok += 1; }
            if v5_matched { v5_ok += 1; }
            if !v4_matched && !v5_matched { both_fail += 1; }

            if name_v4 != name_v5 {
                disagree.push(format!(
                    "  [{:04}] v4={:?} (OCR: {:?})  v5={:?} (OCR: {:?})",
                    idx,
                    name_v4.as_deref().unwrap_or("FAIL"),
                    text_v4.trim(),
                    name_v5.as_deref().unwrap_or("FAIL"),
                    text_v5.trim(),
                ));
            }
        }

        println!("Total: {} characters", entries.len());
        println!("v4 matched: {}/{} ({:.1}%)", v4_ok, entries.len(), v4_ok as f64 / entries.len() as f64 * 100.0);
        println!("v5 matched: {}/{} ({:.1}%)", v5_ok, entries.len(), v5_ok as f64 / entries.len() as f64 * 100.0);
        println!("Both failed: {}", both_fail);
        if !disagree.is_empty() {
            println!("Disagreements ({}):", disagree.len());
            for line in &disagree {
                println!("{}", line);
            }
        }
    }

    // Evaluate weapon equip
    let weapon_dir = base.join("weapons");
    if weapon_dir.is_dir() {
        println!();
        println!("=== Weapon Equip OCR (v4 vs v5) ===");
        let mut entries: Vec<(usize, std::path::PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&weapon_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(idx) = name.parse::<usize>() {
                let path = entry.path().join("equip.png");
                if path.exists() {
                    entries.push((idx, path));
                }
            }
        }
        entries.sort_by_key(|(idx, _)| *idx);

        let mut v4_ok = 0;
        let mut v5_ok = 0;
        let mut both_fail = 0;
        let mut disagree = Vec::new();
        let mut has_text = 0;

        for (idx, path) in &entries {
            let img = image::open(path)?.to_rgb8();
            let text_v4 = v4.image_to_text(&img, false)?;
            let text_v5 = v5.image_to_text(&img, false)?;

            // Skip empty equip slots (no text at all)
            if text_v4.trim().is_empty() && text_v5.trim().is_empty() {
                continue;
            }
            has_text += 1;

            let loc_v4 = equip_parser::parse_equip_location(&text_v4, &mappings.character_name_map);
            let loc_v5 = equip_parser::parse_equip_location(&text_v5, &mappings.character_name_map);

            let v4_matched = !loc_v4.is_empty();
            let v5_matched = !loc_v5.is_empty();
            if v4_matched { v4_ok += 1; }
            if v5_matched { v5_ok += 1; }
            if !v4_matched && !v5_matched { both_fail += 1; }

            if loc_v4 != loc_v5 {
                disagree.push(format!(
                    "  [{:04}] v4={:?} (OCR: {:?})  v5={:?} (OCR: {:?})",
                    idx,
                    if v4_matched { &loc_v4 } else { "FAIL" },
                    text_v4.trim(),
                    if v5_matched { &loc_v5 } else { "FAIL" },
                    text_v5.trim(),
                ));
            }
        }

        println!("Total: {} weapons ({} equipped)", entries.len(), has_text);
        if has_text > 0 {
            println!("v4 matched: {}/{} ({:.1}%)", v4_ok, has_text, v4_ok as f64 / has_text as f64 * 100.0);
            println!("v5 matched: {}/{} ({:.1}%)", v5_ok, has_text, v5_ok as f64 / has_text as f64 * 100.0);
            println!("Both failed: {}", both_fail);
        }
        if !disagree.is_empty() {
            println!("Disagreements ({}):", disagree.len());
            for line in &disagree {
                println!("{}", line);
            }
        }
    }

    // Evaluate artifact equip
    let artifact_dir = base.join("artifacts");
    if artifact_dir.is_dir() {
        println!();
        println!("=== Artifact Equip OCR (v4 vs v5) ===");
        let mut entries: Vec<(usize, std::path::PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&artifact_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(idx) = name.parse::<usize>() {
                let path = entry.path().join("equip.png");
                if path.exists() {
                    entries.push((idx, path));
                }
            }
        }
        entries.sort_by_key(|(idx, _)| *idx);

        let mut v4_ok = 0;
        let mut v5_ok = 0;
        let mut both_fail = 0;
        let mut disagree = Vec::new();
        let mut has_text = 0;

        for (idx, path) in &entries {
            let img = image::open(path)?.to_rgb8();
            let text_v4 = v4.image_to_text(&img, false)?;
            let text_v5 = v5.image_to_text(&img, false)?;

            if text_v4.trim().is_empty() && text_v5.trim().is_empty() {
                continue;
            }
            has_text += 1;

            let loc_v4 = equip_parser::parse_equip_location(&text_v4, &mappings.character_name_map);
            let loc_v5 = equip_parser::parse_equip_location(&text_v5, &mappings.character_name_map);

            let v4_matched = !loc_v4.is_empty();
            let v5_matched = !loc_v5.is_empty();
            if v4_matched { v4_ok += 1; }
            if v5_matched { v5_ok += 1; }
            if !v4_matched && !v5_matched { both_fail += 1; }

            if loc_v4 != loc_v5 {
                disagree.push(format!(
                    "  [{:04}] v4={:?} (OCR: {:?})  v5={:?} (OCR: {:?})",
                    idx,
                    if v4_matched { &loc_v4 } else { "FAIL" },
                    text_v4.trim(),
                    if v5_matched { &loc_v5 } else { "FAIL" },
                    text_v5.trim(),
                ));
            }
        }

        println!("Total: {} artifacts ({} equipped)", entries.len(), has_text);
        if has_text > 0 {
            println!("v4 matched: {}/{} ({:.1}%)", v4_ok, has_text, v4_ok as f64 / has_text as f64 * 100.0);
            println!("v5 matched: {}/{} ({:.1}%)", v5_ok, has_text, v5_ok as f64 / has_text as f64 * 100.0);
            println!("Both failed: {}", both_fail);
        }
        if !disagree.is_empty() {
            println!("Disagreements ({}):", disagree.len());
            for line in &disagree {
                println!("{}", line);
            }
        }
    }

    Ok(())
}

fn run_equip_test(image_path: &str) -> Result<()> {
    let img = image::open(image_path)?.to_rgb8();
    println!("Image: {} ({}x{})", image_path, img.width(), img.height());

    // Load mappings + models
    println!("Loading mappings...");
    let mappings = MappingManager::new(&NameOverrides::default())?;
    println!("  {} characters loaded", mappings.character_name_map.len());

    println!("Loading models...");
    let v4 = create_ocr_model("ppocrv4")?;
    let v5 = create_ocr_model("ppocrv5")?;

    // OCR with both engines
    let text_v4 = v4.image_to_text(&img, false)?;
    let text_v5 = v5.image_to_text(&img, false)?;

    println!();
    println!("=== Equip Pipeline Test ===");
    println!();

    // v4 path
    let loc_v4 = equip_parser::parse_equip_location(&text_v4, &mappings.character_name_map);
    println!("v4 OCR:   {:?}", text_v4);
    println!("v4 match: {:?}", if loc_v4.is_empty() { "(empty)" } else { &loc_v4 });

    // v5 path
    let loc_v5 = equip_parser::parse_equip_location(&text_v5, &mappings.character_name_map);
    println!();
    println!("v5 OCR:   {:?}", text_v5);
    println!("v5 match: {:?}", if loc_v5.is_empty() { "(empty)" } else { &loc_v5 });

    // Combined path (v4 primary, v5 fallback — as scanner does it)
    println!();
    println!("=== Combined (v4 → v5 fallback) ===");
    let final_loc = if !loc_v4.is_empty() {
        println!("v4 matched: {}", loc_v4);
        loc_v4
    } else if !loc_v5.is_empty() {
        println!("v4 failed, v5 fallback matched: {}", loc_v5);
        loc_v5
    } else {
        println!("BOTH FAILED");
        println!("  v4 raw: {:?}", text_v4);
        println!("  v5 raw: {:?}", text_v5);
        String::new()
    };

    if !final_loc.is_empty() {
        println!("Result: {}", final_loc);
    }

    Ok(())
}
