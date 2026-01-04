/// OCR后端类型
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrBackend {
    Yas,
    Paddle,
}

#[derive(Clone, clap::Args)]
pub struct GenshinArtifactScannerConfig {
    /// Items with stars less than this will be ignored
    #[arg(id = "min-star", long = "min-star", help = "最小星级", value_name = "MIN_STAR", default_value_t = 4)]
    pub min_star: i32,

    /// Items with level less than this will be ignored
    #[arg(id = "min-level", long = "min-level", help = "最小等级", value_name = "MIN_LEVEL", default_value_t = 0)]
    pub min_level: i32,

    /// Ignore duplicated items
    #[arg(id = "ignore-dup", long = "ignore-dup", help = "忽略重复物品")]
    pub ignore_dup: bool,

    /// it will output very verbose messages
    #[arg(id = "verbose", long, help = "显示详细信息")]
    pub verbose: bool,

    /// the exact amount to scan
    #[arg(id = "number", long, help = "指定圣遗物数量", value_name = "NUMBER", default_value_t = -1)]
    pub number: i32,

    /// save captured images for debugging
    #[arg(id = "save-images", long = "save-images", help = "保存识别的图片到当前目录用于调试")]
    pub save_images: bool,

    /// 选择OCR后端: yas 或 paddle
    #[arg(long, help = "选择OCR后端: yas 或 paddle", default_value = "yas")]
    pub ocr_backend: String,

    /// 圣遗物数量识别OCR后端: yas 或 paddle，留空则与 ocr_backend 一致
    #[arg(long, help = "圣遗物数量识别OCR后端: yas 或 paddle，留空则与 ocr_backend 一致", default_value = "ppocrv5")]
    pub item_count_ocr_backend: String,

    /// 副词条4单独指定OCR后端: yas 或 paddle，留空则与 ocr_backend 一致
    #[arg(long, help = "副词条4单独指定OCR后端: yas 或 paddle，留空则与 ocr_backend 一致", default_value = "paddlev3")]
    pub substat4_ocr_backend: String,

    /// 每次切换圣遗物后的额外等待时间(ms)
    #[arg(id = "delay", long, help = "每次切换圣遗物后的额外等待时间(ms)", default_value_t = 20)]
    pub delay: u32,
}
