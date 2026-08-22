slint::include_modules!();

use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

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

// fn create_no_camera_image(width: u32, height: u32) -> Image {
//     let mut pixel_data = vec![0u8; (width * height * 3) as usize];
//
//     for chunk in pixel_data.chunks_mut(3) {
//         chunk[0] = 60;
//         chunk[1] = 60;
//         chunk[2] = 60;
//     }
//
//     let border_width = 3;
//     let cross_width = 4;
//
//     for y in 0..height {
//         for x in 0..width {
//             let is_border = x < border_width || x >= width - border_width
//                 || y < border_width || y >= height - border_width;
//             if is_border {
//                 let idx = ((y * width + x) * 3) as usize;
//                 pixel_data[idx] = 180;
//                 pixel_data[idx + 1] = 180;
//                 pixel_data[idx + 2] = 180;
//             }
//         }
//     }
//
//     for y in 0..height {
//         for x in 0..width {
//             let diag1 = (x as i32 - y as i32).abs() < cross_width as i32;
//             let diag2 = (x as i32 - (height as i32 - 1 - y as i32)).abs() < cross_width as i32;
//             if diag1 || diag2 {
//                 let idx = ((y * width + x) * 3) as usize;
//                 pixel_data[idx] = 200;
//                 pixel_data[idx + 1] = 60;
//                 pixel_data[idx + 2] = 60;
//             }
//         }
//     }
//
//     let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&pixel_data, width, height);
//     Image::from_rgb8(buffer)
// }

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

// fn write_session_json(dir: &std::path::Path, mood_id: i32, timestamp: &str) {
//     let name = mood_name(mood_id);
//     let json = format!(
//         "{{\n  \"recorded_at\": \"{}\",\n  \"mood_id\": {},\n  \"mood_name\": \"{}\"\n}}\n",
//         timestamp, mood_id, name
//     );
//     let path = dir.join("mood.json");
//     if let Err(e) = std::fs::write(&path, json) {
//         eprintln!("Failed to write session.json: {}", e);
//     } else {
//         println!("Mood state saved to {:?}", path);
//     }
// }

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

    app.run()
}
