slint::include_modules!();
use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

// Shared menu creation logic for both platforms
fn create_menu() -> (Menu, muda::MenuId) {
    let menu = Menu::new();

    // File menu
    let file_menu = Submenu::new("File", true);
    let open_item = MenuItem::new("Open...", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    let quit_item_id = quit_item.id().clone();

    file_menu.append_items(&[
        &open_item,
        &PredefinedMenuItem::separator(),
        &quit_item,
    ]).unwrap();

    // Edit menu
    let edit_menu = Submenu::new("Edit", true);
    let preferences_item = MenuItem::new("Preferences", true, None);
    edit_menu.append(&preferences_item).unwrap();

    // View menu
    let view_menu = Submenu::new("View", true);
    let fullscreen_item = MenuItem::new("Toggle Fullscreen", true, None);
    view_menu.append(&fullscreen_item).unwrap();

    // Help menu
    let help_menu = Submenu::new("Help", true);
    let about_item = MenuItem::new("About", true, None);
    help_menu.append(&about_item).unwrap();

    // Add all submenus to main menu
    menu.append_items(&[
        &file_menu,
        &edit_menu,
        &view_menu,
        &help_menu,
    ]).unwrap();

    (menu, quit_item_id)
}

// Shared menu event handler for both platforms
fn start_menu_event_handler(quit_item_id: muda::MenuId) {
    std::thread::spawn(move || {
        loop {
            if let Ok(event) = muda::MenuEvent::receiver().try_recv() {
                if event.id == quit_item_id {
                    std::process::exit(0);
                }
                // Handle other menu events here
                // You can add more menu item checks and actions here
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    // Create menu structure (shared between platforms)
    #[allow(unused_variables)] // menu is only used on macOS currently
    let (menu, quit_item_id) = create_menu();

    // Platform-specific menu initialization
    #[cfg(target_os = "macos")]
    {
        // macOS: Use native app menu bar
        menu.init_for_nsapp();
        println!("Native menu bar initialized for macOS");
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: Try to initialize with GTK
        // Note: This requires Slint to be using the GTK backend
        // For now, we'll print a warning that native menus are not fully supported
        // The in-Slint MenuBar component will be used instead
        eprintln!("Warning: Native menu bar integration on Linux is limited.");
        eprintln!("Using in-window menu bar from app.slint instead.");
        eprintln!("To use native GTK menus, additional integration work is required.");

        // We don't initialize the native menu on Linux for now
        // The MenuBar component in app.slint will be displayed instead
    }

    // Start menu event handler (works on macOS, prepared for Linux)
    start_menu_event_handler(quit_item_id);

    // Create and show the window
    let app = AppWindow::new()?;
    app.window().set_maximized(true);

    // Connect mood voting callback
    app.on_mood_voted(|mood_id| {
        let mood_name = match mood_id {
            1 => "Happy",
            2 => "Sad",
            3 => "Angry",
            4 => "Anxious",
            5 => "Calm",
            6 => "Tired",
            7 => "Excited",
            8 => "Depressed",
            9 => "Frustrated",
            10 => "Scared",
            11 => "Neutral",
            12 => "Thoughtful",
            _ => "Unknown",
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        println!("Mood vote recorded: {} (ID: {}) at timestamp: {}", mood_name, mood_id, timestamp);

        // TODO: Save to file or database for analysis
    });

    app.run()
}
