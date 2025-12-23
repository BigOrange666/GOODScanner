/// 去除副词条中的“（”及其后内容
fn clean_stat_text(s: &str) -> String {
    match s.find('（') {
        Some(idx) => s[..idx].trim_end().to_string(),
        None => s.trim().to_string(),
    }
}
use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::path::Path;

use anyhow::Result;
use image::Rgb;
use image::{GenericImageView, RgbImage};
use log::{error, info, warn};

use yas::ocr::ImageToText;
use yas::ocr::yas_ocr_model;
use yas::ocr::PPOCRModel;
use yas::positioning::{Pos, Rect};
use yas::utils::color_distance;

use crate::scanner::artifact_scanner::artifact_scanner_window_info::ArtifactScannerWindowInfo;
use crate::scanner::artifact_scanner::GenshinArtifactScannerConfig;
use crate::scanner::artifact_scanner::message_items::SendItem;
use crate::scanner::artifact_scanner::scan_result::GenshinArtifactScanResult;

fn parse_level(s: &str) -> Result<i32> {
    // 自动容错：将 o/O 替换为 0，只保留数字和正负号
    let replaced = s.replace(['o', 'O'], "0");
    let cleaned: String = replaced.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '+').collect();
    let cleaned = cleaned.trim_start_matches('+');
    if cleaned.is_empty() {
        log::warn!("parse_level 解析失败: 原始内容='{}', o/O->0后为空，自动返回0", s);
        return Ok(0);
    }
    match cleaned.parse::<i32>() {
        Ok(level) => Ok(level),
        Err(e) => {
            log::warn!("parse_level 解析失败: 原始内容='{}', o/O->0后='{}', 错误: {}，自动返回0", s, cleaned, e);
            Ok(0)
        }
    }
}

fn get_image_to_text(backend: &str) -> Result<Box<dyn ImageToText<RgbImage> + Send>> {
    match backend.to_lowercase().as_str() {
        "paddle" | "ppocrv5" => {
            // PaddleOCR v5模型，内嵌模型和字典
            let model_bytes = include_bytes!("./models/PP-OCRv5_mobile_rec.onnx");
            let dict_str = include_str!("./models/ppocrv5_dict.txt");
            let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.to_string()).collect();
            // PaddleOCR 字典需要在末尾添加空格字符
            dict_vec.push(String::from(" "));
            let model = PPOCRModel::new(model_bytes, dict_vec)?;
            Ok(Box::new(model))
        },
        "paddlev3" | "ppocrv3" => {
            // PaddleOCR v3模型，内嵌模型和字典
            let model_bytes = include_bytes!("./models/ch_PP-OCRv3_rec_infer.onnx");
            let dict_str = include_str!("./models/ppocr_keys_v1.txt");
            let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.to_string()).collect();
            // PaddleOCR 字典需要在末尾添加空格字符
            dict_vec.push(String::from(" "));
            let model = PPOCRModel::new(model_bytes, dict_vec)?;
            Ok(Box::new(model))
        },
        _ => {
            let model: Box<dyn ImageToText<RgbImage> + Send> = Box::new(
                yas_ocr_model!("./models/model_training.onnx", "./models/index_2_word.json")?
            );
            Ok(model)
        }
    }
}

/// Save image for debugging purposes
fn save_debug_image(image: &RgbImage, artifact_index: usize, region_tag: &str) -> Result<()> {
    // Create debug directory if it doesn't exist
    let debug_dir = Path::new("./debug_images");
    if !debug_dir.exists() {
        std::fs::create_dir_all(debug_dir)?;
    }

    let full_filename = format!("artifact_{}_{}.png", artifact_index, region_tag);
    let image_path = debug_dir.join(full_filename);

    image.save(&image_path)?;
    info!("Debug image saved: {}", image_path.display());

    Ok(())
}

/// run in a separate thread, accept captured image and get an artifact
pub struct ArtifactScannerWorker {
    model: Box<dyn ImageToText<RgbImage> + Send>,
    paddle_model: Option<Box<dyn ImageToText<RgbImage> + Send>>,
    paddlev3_model: Option<Box<dyn ImageToText<RgbImage> + Send>>,  // PaddleOCR v3 模型
    yas_model: Option<Box<dyn ImageToText<RgbImage> + Send>>,
    window_info: ArtifactScannerWindowInfo,
    config: GenshinArtifactScannerConfig,
}

impl ArtifactScannerWorker {
    fn get_model_for_backend(backend: &str) -> Result<Box<dyn ImageToText<RgbImage> + Send>> {
        match backend.to_lowercase().as_str() {
            "paddle" | "ppocrv5" => {
                let model_bytes = include_bytes!("./models/PP-OCRv5_mobile_rec.onnx");
                let dict_str = include_str!("./models/ppocrv5_dict.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.to_string()).collect();
                // PaddleOCR 字典需要在末尾添加空格字符
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            },
            "paddlev3" | "ppocrv3" => {
                let model_bytes = include_bytes!("./models/ch_PP-OCRv3_rec_infer.onnx");
                let dict_str = include_str!("./models/ppocr_keys_v1.txt");
                let mut dict_vec: Vec<String> = dict_str.lines().map(|l| l.to_string()).collect();
                // PaddleOCR 字典需要在末尾添加空格字符
                dict_vec.push(String::from(" "));
                let model = PPOCRModel::new(model_bytes, dict_vec)?;
                Ok(Box::new(model))
            },
            _ => {
                let model: Box<dyn ImageToText<RgbImage> + Send> = Box::new(
                    yas_ocr_model!("./models/model_training.onnx", "./models/index_2_word.json")?
                );
                Ok(model)
            }
        }
    }
    pub fn new(
        window_info: ArtifactScannerWindowInfo,
        config: GenshinArtifactScannerConfig,
    ) -> Result<Self> {
        let model = get_image_to_text(&config.ocr_backend)?;
        
        // paddle_model 用于 title 识别,始终加载以确保 title 使用 PaddleOCR v5
        let paddle_model = Some(Self::get_model_for_backend("paddle")?);
        
        let paddlev3_model = if config.ocr_backend.to_lowercase() == "paddlev3" 
            || config.ocr_backend.to_lowercase() == "ppocrv3"
            || config.substat4_ocr_backend.to_lowercase() == "paddlev3" 
            || config.substat4_ocr_backend.to_lowercase() == "ppocrv3" {
            Some(Self::get_model_for_backend("paddlev3")?)
        } else {
            None
        };
        
        let yas_model = if config.ocr_backend.to_lowercase() == "yas" || config.substat4_ocr_backend.to_lowercase() == "yas" {
            Some(Self::get_model_for_backend("yas")?)
        } else {
            None
        };
        
        Ok(ArtifactScannerWorker {
            model,
            paddle_model,
            paddlev3_model,
            yas_model,
            window_info,
            config,
        })
    }

    /// the captured_img is a panel of the artifact, the rect is a region of the panel
    fn model_inference(&self, rect: Rect<f64>, captured_img: &RgbImage, artifact_index: usize, region_tag: &str) -> Result<String> {
        let relative_rect = rect.translate(Pos {
            x: -self.window_info.panel_rect.left,
            y: -self.window_info.panel_rect.top,
        });

        let raw_img = captured_img.view(
            relative_rect.left as u32, relative_rect.top as u32, relative_rect.width as u32, relative_rect.height as u32,
        ).to_image();

        if self.config.save_images {
            if let Err(e) = save_debug_image(&raw_img, artifact_index, region_tag) {
                warn!("Failed to save region debug image: {}", e);
            }
        }

        let inference_result = self.model.image_to_text(&raw_img, false);
        
        // 添加错误上下文信息
        match inference_result {
            Ok(text) => Ok(text),
            Err(e) => {
                error!("OCR识别失败: artifact_index={}, region_tag='{}', 错误: {:?}", artifact_index, region_tag, e);
                Err(e)
            }
        }
    }

    /// Detect if the artifact has "祝圣" text which shifts all stats down by one line
    fn detect_consecration_shift(&self, image: &RgbImage, artifact_index: usize) -> f64 {
        // 检测区域：在 main_stat_value 和 sub_stat_1 之间（祝圣文本应该出现在这里）
        // 使用一个小的检测区域来识别是否有"祝圣"关键字
        // 向下偏移一个词条高度来定位祝圣文本的实际位置
        let substat_line_height = self.window_info.sub_stat_2.top - self.window_info.sub_stat_1.top;
        let detect_rect = Rect {
            left: self.window_info.main_stat_value_rect.left,
            top: self.window_info.main_stat_value_rect.top + self.window_info.main_stat_value_rect.height + substat_line_height + self.window_info.item_equip_rect.height,
            width: self.window_info.main_stat_value_rect.width,
            height: substat_line_height, // 检测一个小区域高度（祝圣文本大约一行的高度）
        };
        
        let relative_rect = detect_rect.translate(Pos {
            x: -self.window_info.panel_rect.left,
            y: -self.window_info.panel_rect.top,
        });
        
        // 安全检查：确保区域在图像范围内
        if relative_rect.left < 0.0 || relative_rect.top < 0.0 
            || (relative_rect.left + relative_rect.width) as u32 > image.width()
            || (relative_rect.top + relative_rect.height) as u32 > image.height() {
            return 0.0;
        }
        
        let raw_img = image.view(
            relative_rect.left as u32,
            relative_rect.top as u32,
            relative_rect.width as u32,
            relative_rect.height as u32,
        ).to_image();
        
        if self.config.save_images {
            if let Err(e) = save_debug_image(&raw_img, artifact_index, "consecration_detect") {
                warn!("Failed to save consecration detection image: {}", e);
            }
        }
        
        // 使用 paddle 模型识别（更准确）
        if let Some(model) = &self.paddle_model {
            match model.image_to_text(&raw_img, false) {
                Ok(text) => {
                    let normalized = text.chars().filter(|c| !c.is_whitespace()).collect::<String>();
                    // 检测祝圣关键字（各种可能的OCR结果）
                    if normalized.contains("祝圣") || normalized.contains("之霜") || normalized.contains("之油") 
                        || normalized.contains("之露") || normalized.contains("之葩") || normalized.contains("定义") {
                        if self.config.verbose {
                            info!("[artifact_index={}] 检测到祝圣文本: '{}', 应用位置偏移", artifact_index, text);
                        }
                        // 返回偏移量：大约一个副词条的高度
                        return self.window_info.sub_stat_2.top - self.window_info.sub_stat_1.top;
                    }
                },
                Err(e) => {
                    warn!("OCR识别失败: artifact_index={}, region_tag='consecration_detect', 错误: {:?}", artifact_index, e);
                }
            }
        }
        
        0.0 // 没有检测到祝圣文本，不偏移
    }

    /// Parse the captured result (of type SendItem) to a scanned artifact
    fn scan_item_image(&self, item: SendItem, lock: bool, artifact_index: usize) -> Result<GenshinArtifactScanResult> {
        let image = &item.panel_image;
        let str_main_stat_name = self.model_inference(self.window_info.main_stat_name_rect, image, artifact_index, "main_stat_name")?;
        let str_main_stat_value = self.model_inference(self.window_info.main_stat_value_rect, image, artifact_index, "main_stat_value")?;
        
        // 检测是否有祝圣文本，如果有则计算需要向下偏移的距离
        let shift_offset = self.detect_consecration_shift(image, artifact_index);
        
        // 应用偏移后的副词条区域
        let sub_stat_1_rect = self.window_info.sub_stat_1.translate(Pos { x: 0.0, y: shift_offset });
        let sub_stat_2_rect = self.window_info.sub_stat_2.translate(Pos { x: 0.0, y: shift_offset });
        let sub_stat_3_rect = self.window_info.sub_stat_3.translate(Pos { x: 0.0, y: shift_offset });
        let sub_stat_4_rect = self.window_info.sub_stat_4.translate(Pos { x: 0.0, y: shift_offset });
        let level_rect = self.window_info.level_rect.translate(Pos { x: 0.0, y: shift_offset });
        
        let str_sub_stat0 = clean_stat_text(&self.model_inference(sub_stat_1_rect, image, artifact_index, "substat1")?);
        let str_sub_stat1 = clean_stat_text(&self.model_inference(sub_stat_2_rect, image, artifact_index, "substat2")?);

        // Save debug image if enabled
        if self.config.save_images {
            // if let Err(e) = save_debug_image(image, artifact_index, "panel") {
            //     warn!("Failed to save debug image: {}", e);
            // }
        }

        let str_title = {
            // 使用 PaddleOCR v5 模型识别 title (圣遗物名称)
            let model: &Box<dyn ImageToText<RgbImage> + Send> = self.paddle_model.as_ref().expect("paddle_model should be initialized for title recognition");
            let relative_rect = self.window_info.title_rect.translate(Pos {
                x: -self.window_info.panel_rect.left,
                y: -self.window_info.panel_rect.top,
            });
            let raw_img = image.view(
                relative_rect.left as u32, relative_rect.top as u32, relative_rect.width as u32, relative_rect.height as u32,
            ).to_image();
            
            if self.config.save_images {
                if let Err(e) = save_debug_image(&raw_img, artifact_index, "title") {
                    warn!("Failed to save region debug image: {}", e);
                }
            }
            
            model.image_to_text(&raw_img, false)?
        };
        let str_sub_stat3 = if !self.config.substat4_ocr_backend.is_empty() {
            let backend = self.config.substat4_ocr_backend.to_lowercase();
            let model: &Box<dyn ImageToText<RgbImage> + Send> = match backend.as_str() {
                "paddle" | "ppocrv5" => self.paddle_model.as_ref().expect("paddle_model should be initialized"),
                "paddlev3" | "ppocrv3" => self.paddlev3_model.as_ref().expect("paddlev3_model should be initialized"),
                "yas" => self.yas_model.as_ref().expect("yas_model should be initialized"),
                _ => &self.model,
            };
            let relative_rect = sub_stat_4_rect.translate(Pos {
                x: -self.window_info.panel_rect.left,
                y: -self.window_info.panel_rect.top,
            });
            let raw_img = image.view(
                relative_rect.left as u32, relative_rect.top as u32, relative_rect.width as u32, relative_rect.height as u32,
            ).to_image();
            
            if self.config.save_images {
                if let Err(e) = save_debug_image(&raw_img, artifact_index, "substat4") {
                    warn!("Failed to save region debug image: {}", e);
                }
            }
            
            let raw = model.image_to_text(&raw_img, false)?;
            clean_stat_text(&raw)
        } else {
            let raw = self.model_inference(sub_stat_4_rect, image, artifact_index, "substat4")?;
            clean_stat_text(&raw)
        };
        let str_sub_stat2 = clean_stat_text(&self.model_inference(sub_stat_3_rect, image, artifact_index, "substat3")?);
        let str_level = self.model_inference(level_rect, image, artifact_index, "level")?;
        let str_equip = {
            // 使用 PaddleOCR v5 模型识别装备者，减少繁/简体和错字识别差异
            let relative_rect = self.window_info.item_equip_rect.translate(Pos {
                x: -self.window_info.panel_rect.left,
                y: -self.window_info.panel_rect.top,
            });
            let raw_img = image.view(
                relative_rect.left as u32,
                relative_rect.top as u32,
                relative_rect.width as u32,
                relative_rect.height as u32,
            ).to_image();

            if self.config.save_images {
                if let Err(e) = save_debug_image(&raw_img, artifact_index, "equip") {
                    warn!("Failed to save region debug image: {}", e);
                }
            }

            let model: &Box<dyn ImageToText<RgbImage> + Send> = self.paddle_model.as_ref().expect("paddle_model should be initialized for equip recognition");
            // Resize equip region to fixed size (320x48) to match Python preprocessing, then mark as preprocessed
            let fixed = image::imageops::resize(&raw_img, 320, 48, image::imageops::FilterType::Triangle);
            model.image_to_text(&fixed, true)?
        };

        anyhow::Ok(GenshinArtifactScanResult {
            name: str_title,
            main_stat_name: str_main_stat_name,
            main_stat_value: str_main_stat_value,
            sub_stat: [
                str_sub_stat0,
                str_sub_stat1,
                str_sub_stat2,
                str_sub_stat3,
            ],
            level: parse_level(&str_level)?,
            equip: str_equip,
            star: item.star as i32,
            lock,
            index: artifact_index,
        })
    }

    /// Get all lock state from a list image
    fn get_page_locks(&self, list_image: &RgbImage) -> Vec<bool> {
        let mut result = Vec::new();

        let row = self.window_info.row;
        let col = self.window_info.col;
        let gap = self.window_info.item_gap_size;
        let size = self.window_info.item_size;
        let lock_pos = self.window_info.lock_pos;

        for r in 0..row {
            if ((gap.height + size.height) * (r as f64)) as u32 > list_image.height() {
                break;
            }
            for c in 0..col {
                let pos_x = (gap.width + size.width) * (c as f64) + lock_pos.x;
                let pos_y = (gap.height + size.height) * (r as f64) + lock_pos.y;

                let mut locked = false;
                'sq: for dx in -1..1 {
                    for dy in -10..10 {
                        if pos_y as i32 + dy < 0 || (pos_y as i32 + dy) as u32 >= list_image.height() {
                            continue;
                        }

                        let color = list_image
                            .get_pixel((pos_x as i32 + dx) as u32, (pos_y as i32 + dy) as u32);

                        if color_distance(color, &Rgb([255, 138, 117])) < 30 {
                            locked = true;
                            break 'sq;
                        }
                    }
                }
                result.push(locked);
            }
        }
        result
    }

    pub fn run(self, rx: Receiver<Option<SendItem>>) -> JoinHandle<Vec<GenshinArtifactScanResult>> {
        std::thread::spawn(move || {
            let mut results = Vec::new();
            let mut hash: HashSet<GenshinArtifactScanResult> = HashSet::new();
            // if too many artifacts are same in consecutive, then an error has occurred
            let mut consecutive_dup_count = 0;

            let is_verbose = self.config.verbose;
            let min_level = self.config.min_level;
            let info = self.window_info.clone();
            // todo remove dump mode to another scanner
            // let dump_mode = false;
            // let model = self.model.clone();
            // let panel_origin = Pos { x: self.window_info.panel_rect.left, y: self.window_info.panel_rect.top };

            let mut locks = Vec::new();
            let mut artifact_index: i32 = 0;

            for item in rx.into_iter() {
                // receiving None, which means the worker should end
                let item = match item {
                    Some(v) => v,
                    None => break,
                };

                // if there is a list image, then parse the lock state
                match item.list_image.as_ref() {
                    Some(v) => {
                        locks = vec![locks, self.get_page_locks(v)].concat()
                    }
                    None => {}
                };

                artifact_index += 1;
                let lock_flag = if (artifact_index as usize - 1) < locks.len() {
                    locks[artifact_index as usize - 1]
                } else {
                    error!(
                        "locks 越界：artifact_index={}，locks.len()={}，本次物品将视为未锁定。请检查背包截图或窗口参数。",
                        artifact_index, locks.len()
                    );
                    false
                };
                let result = match self.scan_item_image(item, lock_flag, artifact_index as usize) {
                    Ok(v) => v,
                    Err(e) => {
                        error!("识别错误: {}", e);
                        continue;
                    }
                };


                if is_verbose {
                    info!("{:?}", result);
                }
                // 尝试转换为 GenshinArtifact，若失败输出详细原因
                if let Err(e) = crate::artifact::GenshinArtifact::try_from(&result) {
                    error!(
                        "artifact_index={} GenshinArtifact::try_from 失败，name='{}', main_stat_name='{}', main_stat_value='{}', sub_stat4='{}'，原因: {:?}",
                        artifact_index,
                        result.name,
                        result.main_stat_name,
                        result.main_stat_value,
                        result.sub_stat[3],
                        e
                    );
                }

                if result.level < min_level {
                    info!(
                        "扫描终止：找到满足最低等级要求 {} 的物品({})，已扫描 {} 个。",
                        min_level, result.level, artifact_index
                    );
                    break;
                }

                if hash.contains(&result) {
                    consecutive_dup_count += 1;
                    warn!("识别到重复物品: {:#?}", result);
                } else {
                    consecutive_dup_count = 0;
                    hash.insert(result.clone());
                    results.push(result);
                }

                if consecutive_dup_count >= info.col && !self.config.ignore_dup {
                    error!(
                        "扫描终止：识别到连续 {} 个重复物品，可能为翻页错误，或者为非背包顶部开始扫描，已扫描 {} 个。",
                        consecutive_dup_count, artifact_index
                    );
                    break;
                }

                // if token.cancelled() {
                // error!("扫描任务被取消");
                // break;
                // }
            }

            info!("识别结束，非重复物品数量: {}", hash.len());

            // progress_bar.finish();
            // MULTI_PROGRESS.remove(&progress_bar);

            results
        })
    }
}
