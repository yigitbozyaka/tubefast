#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod art;
mod auth;
mod discord;
mod media;
mod player;
mod ui;
mod update;
mod ytm;

use eframe::egui;
use std::io::Write;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const APP_NAME: &str = "Tubefast";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_ID: &str = "tubefast";
const CRASH_LOG: &str = "crash.log";
const CRASH_LOG_BYTES: u64 = 64 * 1024;
const PROFILE_FLAG: &str = "--profile";
const PREVIOUS_VERSION_EXIT: Duration = Duration::from_millis(800);
static PROFILE: OnceLock<String> = OnceLock::new();

pub fn app_id() -> &'static str {
    PROFILE.get().map_or(APP_ID, String::as_str)
}

fn take_profile(arguments: &mut Vec<String>) {
    let Some(at) = arguments.iter().position(|argument| argument == PROFILE_FLAG) else {
        return;
    };
    let name = arguments.drain(at..(at + 2).min(arguments.len())).nth(1).unwrap_or_default();
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        let _ = PROFILE.set(format!("{APP_ID}-{name}"));
    }
}

pub fn profile_arguments() -> Vec<String> {
    let name = PROFILE.get().and_then(|id| id.strip_prefix(APP_ID)?.strip_prefix('-'));
    name.map(|name| vec![PROFILE_FLAG.to_owned(), name.to_owned()]).unwrap_or_default()
}

fn take_updated(arguments: &mut Vec<String>) -> bool {
    let before = arguments.len();
    arguments.retain(|argument| argument != update::UPDATED_FLAG);
    arguments.len() < before
}

#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}

fn log_crashes() {
    let Some(folder) = eframe::storage_dir(app_id()) else { return };
    std::panic::set_hook(Box::new(move |panic| {
        let file = folder.join(CRASH_LOG);
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
        let grown = std::fs::metadata(&file).is_ok_and(|log| log.len() > CRASH_LOG_BYTES);
        let _ = std::fs::create_dir_all(&folder);
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(!grown)
            .write(true)
            .truncate(grown)
            .open(file);
        if let Ok(mut log) = log {
            let _ = writeln!(log, "{APP_NAME} {VERSION} at unix {seconds}: {panic}");
        }
    }));
}

fn main() -> eframe::Result {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    take_profile(&mut arguments);
    log_crashes();
    if take_updated(&mut arguments) {
        std::thread::sleep(PREVIOUS_VERSION_EXIT);
    }
    update::clear_leftovers();
    let mut arguments = arguments.into_iter();
    let (flag, value) = (arguments.next(), arguments.next());
    if flag.as_deref() == Some("--selftest") {
        attach_console();
        match player::selftest(value.as_deref().unwrap_or("daft punk get lucky")) {
            Ok(report) => println!("{report}"),
            Err(error) => {
                eprintln!("selftest failed: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let (query, autoplay) = match (flag, value) {
        (Some(flag), Some(query)) if flag == "--play" => (Some(query), true),
        (Some(query), None) if !query.starts_with("--") => (Some(query), false),
        _ => (None, false),
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id(app_id())
        .with_inner_size(app::WINDOW_SIZE)
        .with_min_inner_size(app::WINDOW_MIN)
        .with_icon(ui::app_icon());
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, query, autoplay)))),
    )
}
