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