use yas::utils::press_any_key_to_continue;
use yas_genshin::application::ArtifactScannerApplication;
use log::error;

pub fn main() {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    let command = ArtifactScannerApplication::build_command();
    let matches = match command.try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            // 打印错误或帮助信息
            eprintln!("{}", e);
            press_any_key_to_continue();
            std::process::exit(if e.use_stderr() { 1 } else { 0 });
        }
    };

    let application = ArtifactScannerApplication::new(matches);
    match application.run() {
        Err(e) => {
            error!("error: {}", e);
            press_any_key_to_continue();
        },
        _ => {
            press_any_key_to_continue();
        }
    }
}