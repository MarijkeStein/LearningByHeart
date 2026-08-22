use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

pub fn create_menu() -> (Menu, muda::MenuId) {
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

pub fn start_menu_event_handler(quit_item_id: muda::MenuId) {
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

pub fn mood_name(id: i32) -> &'static str {
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