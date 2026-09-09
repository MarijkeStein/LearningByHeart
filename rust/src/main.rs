mod audio;
mod gui;
mod heartbeat;
mod recording;
mod video;

slint::include_modules!();

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const LBH_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<(), slint::PlatformError> {
    println!("LBH version {} on {}", LBH_VERSION, std::env::consts::OS);

    let app = AppWindow::new()?;
    app.window().set_maximized(true);

    app.on_mood_voted(|mood_id| {
        println!("Mood selected: {} (ID: {})", gui::mood_name(mood_id), mood_id);
    });

    // Query audio format before spawning threads so recording knows the WAV spec.
    let audio_format = audio::query_default_audio_format();

    // Shared recording sinks — None when idle, Some(sender) when recording.
    let video_sink: recording::VideoSink = Arc::new(Mutex::new(None));
    let audio_sink: recording::AudioSink = Arc::new(Mutex::new(None));
    let hr_sink:    recording::HrSink    = Arc::new(Mutex::new(None));

    // Start webcam preview.
    let cam_active = Arc::new(AtomicBool::new(app.get_video_enabled()));
    video::start_webcam_preview(app.as_weak(), cam_active.clone(), video_sink.clone());
    app.on_video_enabled_changed(move |enabled| {
        cam_active.store(enabled, Ordering::Relaxed);
    });

    // Start microphone preview.
    let mic_active = Arc::new(AtomicBool::new(app.get_microphone_enabled()));
    audio::start_audio_preview(app.as_weak(), mic_active.clone(), audio_sink.clone());
    app.on_microphone_enabled_changed(move |enabled| {
        mic_active.store(enabled, Ordering::Relaxed);
    });

    // Start Bluetooth heart rate monitor.
    heartbeat::start_heartbeat_monitor(app.as_weak(), hr_sink.clone());

    // Active recording state — Some while a recording is in progress.
    let active_rec: Arc<Mutex<Option<recording::ActiveRecording>>> = Arc::new(Mutex::new(None));

    // Wire up Start Recording.
    {
        let active_rec  = active_rec.clone();
        let video_sink  = video_sink.clone();
        let audio_sink  = audio_sink.clone();
        let hr_sink     = hr_sink.clone();
        let app_weak    = app.as_weak();

        app.on_start_recording(move || {
            let Some(app) = app_weak.upgrade() else { return };

            // Guard against double-start.
            if active_rec.lock().unwrap().is_some() {
                return;
            }

            let video_enabled = app.get_video_enabled();
            let audio_enabled = app.get_microphone_enabled();

            let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
            let dir = std::path::PathBuf::from("../recordings").join(&timestamp);

            let (sample_rate, channels) = audio_format.unwrap_or((44100, 2));

            match recording::start_recording(
                dir,
                &video_sink,
                &audio_sink,
                sample_rate,
                channels,
                &hr_sink,
                video_enabled,
                audio_enabled,
            ) {
                Ok(rec) => {
                    eprintln!("Recording started: {}", rec.dir.display());
                    *active_rec.lock().unwrap() = Some(rec);
                    app.set_is_recording(true);
                }
                Err(e) => {
                    eprintln!("Failed to start recording: {e}");
                }
            }
        });
    }

    // Wire up Stop Recording.
    {
        let active_rec = active_rec.clone();
        let video_sink = video_sink.clone();
        let audio_sink = audio_sink.clone();
        let hr_sink    = hr_sink.clone();
        let app_weak   = app.as_weak();

        app.on_stop_recording(move || {
            if let Some(rec) = active_rec.lock().unwrap().take() {
                eprintln!("Recording stopped: {}", rec.dir.display());
                recording::stop_recording(rec, &video_sink, &audio_sink, &hr_sink);
            }
            if let Some(app) = app_weak.upgrade() {
                app.set_is_recording(false);
            }
        });
    }

    app.run()
}
