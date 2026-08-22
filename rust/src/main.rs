slint::include_modules!();

use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

fn create_menu() -> (Menu, muda::MenuId) {
    let menu = Menu::new();

    let file_menu = Submenu::new("File", true);
    let open_item = MenuItem::new("Open...", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    let quit_item_id = quit_item.id().clone();
    file_menu.append_items(&[&open_item, &PredefinedMenuItem::separator(), &quit_item]).unwrap();

    let edit_menu = Submenu::new("Edit", true);
    let preferences_item = MenuItem::new("Preferences", true, None);
    edit_menu.append(&preferences_item).unwrap();

    let view_menu = Submenu::new("View", true);
    let fullscreen_item = MenuItem::new("Toggle Fullscreen", true, None);
    view_menu.append(&fullscreen_item).unwrap();

    let help_menu = Submenu::new("Help", true);
    let about_item = MenuItem::new("About", true, None);
    help_menu.append(&about_item).unwrap();

    menu.append_items(&[&file_menu, &edit_menu, &view_menu, &help_menu]).unwrap();

    (menu, quit_item_id)
}

fn start_menu_event_handler(quit_item_id: muda::MenuId) {
    std::thread::spawn(move || {
        loop {
            if let Ok(event) = muda::MenuEvent::receiver().try_recv() {
                if event.id == quit_item_id {
                    std::process::exit(0);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

fn mood_name(id: i32) -> &'static str {
    match id {
        1 => "Excited",
        2 => "In Love",
        3 => "Partying",
        4 => "Happy",
        5 => "Neutral",
        6 => "Tired",
        7 => "Exploding",
        8 => "Fearful",
        9 => "Sad",
        10 => "Unwell",
        11 => "Sick",
        12 => "None/Other",
        _ => "Not selected",
    }
}

/// Spawns a background thread that continuously captures frames from the default
/// webcam and pushes them to the UI via Slint's event loop.
///
/// `capture_active` controls whether the thread actually grabs frames; when false
/// the thread sleeps cheaply, allowing the sensor to stay warm for a fast resume.
/// The thread exits automatically once the Slint event loop is gone.
fn start_webcam_preview(app_weak: slint::Weak<AppWindow>, capture_active: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        // Request 640×480 MJPEG at 15 fps — widely supported and low-energy.
        // nokhwa picks the closest available format if the camera can't match exactly.
        let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
            CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 15),
        ));

        let mut camera = match nokhwa::Camera::new(CameraIndex::Index(0), requested) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Webcam unavailable: {e}");
                // Uncheck the camera toggle so the UI shows "Camera disabled"
                let _ = slint::invoke_from_event_loop({
                    let weak = app_weak.clone();
                    move || {
                        if let Some(app) = weak.upgrade() {
                            app.set_video_enabled(false);
                        }
                    }
                });
                return;
            }
        };

        if let Err(e) = camera.open_stream() {
            eprintln!("Failed to open webcam stream: {e}");
            let _ = slint::invoke_from_event_loop({
                let weak = app_weak.clone();
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_video_enabled(false);
                    }
                }
            });
            return;
        }

        // Cap at 15 fps regardless of what the camera actually runs at.
        let frame_interval = std::time::Duration::from_millis(67);

        loop {
            if !capture_active.load(Ordering::Relaxed) {
                // Sleep cheaply; sensor stays warm for instant resume.
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }

            let t = std::time::Instant::now();

            match camera.frame() {
                Ok(frame) => {
                    match frame.decode_image::<RgbFormat>() {
                        Ok(decoded) => {
                            let width = decoded.width();
                            let height = decoded.height();
                            let raw: Vec<u8> = decoded.into_raw();

                            let weak = app_weak.clone();
                            let result = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    let buf = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                                        &raw, width, height,
                                    );
                                    app.set_webcam_preview(Image::from_rgb8(buf));
                                }
                            });
                            if result.is_err() {
                                break; // Event loop is gone — app is closing.
                            }
                        }
                        Err(e) => eprintln!("Webcam decode error: {e}"),
                    }
                }
                Err(e) => eprintln!("Webcam capture error: {e}"),
            }

            let elapsed = t.elapsed();
            if elapsed < frame_interval {
                std::thread::sleep(frame_interval - elapsed);
            }
        }

        let _ = camera.stop_stream();
    });
}

fn main() -> Result<(), slint::PlatformError> {
    println!("LBH version {} on {}", LBH_VERSION, std::env::consts::OS);
    #[allow(unused_variables)]
    let (menu, quit_item_id) = create_menu();

    #[cfg(target_os = "macos")]
    {
        menu.init_for_nsapp();
    }

    start_menu_event_handler(quit_item_id);

    let app = AppWindow::new()?;
    app.window().set_maximized(true);

    app.on_mood_voted(|mood_id| {
        println!("Mood selected: {} (ID: {})", mood_name(mood_id), mood_id);
    });

    // Start webcam preview; initial state mirrors the UI default (video-enabled = true).
    let capture_active = Arc::new(AtomicBool::new(app.get_video_enabled()));
    start_webcam_preview(app.as_weak(), capture_active.clone());

    app.on_video_enabled_changed(move |enabled| {
        capture_active.store(enabled, Ordering::Relaxed);
    });

    app.run()
}
