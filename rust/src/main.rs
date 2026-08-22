mod video;
mod audio;
mod gui;

slint::include_modules!();

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Held for the duration of a recording; dropping it closes the channels and
// signals the writer threads to finalize their files.
#[allow(dead_code)]
#[derive(Clone)]
struct Recording {
    #[allow(dead_code)]
    dir: std::path::PathBuf,
}

// Cargo passes settings from Cargo.toml as env. variable to compiler
const LBH_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<(), slint::PlatformError> {
    println!("LBH version {} on {}", LBH_VERSION, std::env::consts::OS);
    #[allow(unused_variables)]
    let (menu, quit_item_id) = gui::create_menu();

    #[cfg(target_os = "macos")]
    {
        menu.init_for_nsapp();
    }

    gui::start_menu_event_handler(quit_item_id);

    let app = AppWindow::new()?;
    app.window().set_maximized(true);

    app.on_mood_voted(|mood_id| {
        println!("Mood selected: {} (ID: {})", gui::mood_name(mood_id), mood_id);
    });

    // Start webcam preview; initial state mirrors the UI default (video-enabled = true).
    let cam_active = Arc::new(AtomicBool::new(app.get_video_enabled()));
    video::start_webcam_preview(app.as_weak(), cam_active.clone());
    app.on_video_enabled_changed(move |enabled| {
        cam_active.store(enabled, Ordering::Relaxed);
    });

    // Start microphone preview.
    let mic_active = Arc::new(AtomicBool::new(app.get_microphone_enabled()));
    audio::start_audio_preview(app.as_weak(), mic_active.clone());
    app.on_microphone_enabled_changed(move |enabled| {
        mic_active.store(enabled, Ordering::Relaxed);
    });

    app.run()
}
