slint::include_modules!();

use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct RawFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

struct AudioInfo {
    sample_rate: u32,
    channels: u16,
}

// Held for the duration of a recording; dropping it closes the channels and
// signals the writer threads to finalize their files.
#[derive(Clone)]
struct Recording {
    #[allow(dead_code)]
    dir: std::path::PathBuf,
    audio_tx: Option<std::sync::mpsc::SyncSender<Vec<f32>>>,
    video_tx: Option<std::sync::mpsc::SyncSender<RawFrame>>,
}

fn create_no_camera_image(width: u32, height: u32) -> Image {
    let mut pixel_data = vec![0u8; (width * height * 3) as usize];

    for chunk in pixel_data.chunks_mut(3) {
        chunk[0] = 60;
        chunk[1] = 60;
        chunk[2] = 60;
    }

    let border_width = 3;
    let cross_width = 4;

    for y in 0..height {
        for x in 0..width {
            let is_border = x < border_width || x >= width - border_width
                || y < border_width || y >= height - border_width;
            if is_border {
                let idx = ((y * width + x) * 3) as usize;
                pixel_data[idx] = 180;
                pixel_data[idx + 1] = 180;
                pixel_data[idx + 2] = 180;
            }
        }
    }

    for y in 0..height {
        for x in 0..width {
            let diag1 = (x as i32 - y as i32).abs() < cross_width as i32;
            let diag2 = (x as i32 - (height as i32 - 1 - y as i32)).abs() < cross_width as i32;
            if diag1 || diag2 {
                let idx = ((y * width + x) * 3) as usize;
                pixel_data[idx] = 200;
                pixel_data[idx + 1] = 60;
                pixel_data[idx + 2] = 60;
            }
        }
    }

    let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&pixel_data, width, height);
    Image::from_rgb8(buffer)
}

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

fn write_session_json(dir: &std::path::Path, mood_id: i32, timestamp: &str) {
    let name = mood_name(mood_id);
    let json = format!(
        "{{\n  \"recorded_at\": \"{}\",\n  \"mood_id\": {},\n  \"mood_name\": \"{}\"\n}}\n",
        timestamp, mood_id, name
    );
    let path = dir.join("mood.json");
    if let Err(e) = std::fs::write(&path, json) {
        eprintln!("Failed to write session.json: {}", e);
    } else {
        println!("Mood state saved to {:?}", path);
    }
}

fn spawn_audio_writer(
    dir: &std::path::Path,
    sample_rate: u32,
    channels: u16,
) -> std::sync::mpsc::SyncSender<Vec<f32>> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);
    let audio_path = dir.join("audio.wav");

    std::thread::spawn(move || {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = match hound::WavWriter::create(&audio_path, spec) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Failed to create WAV file {:?}: {}", audio_path, e);
                return;
            }
        };
        while let Ok(samples) = rx.recv() {
            for s in samples {
                if let Err(e) = writer.write_sample(s) {
                    eprintln!("Audio write error: {}", e);
                    return;
                }
            }
        }
        if let Err(e) = writer.finalize() {
            eprintln!("Failed to finalize WAV: {}", e);
        } else {
            println!("Audio saved to {:?}", audio_path);
        }
    });

    tx
}

fn spawn_video_writer(dir: &std::path::Path) -> std::sync::mpsc::SyncSender<RawFrame> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<RawFrame>(8);
    let video_path = dir.join("video.mp4");

    std::thread::spawn(move || {
        let first_frame = match rx.recv() {
            Ok(f) => f,
            Err(_) => {
                println!("Video recording: no frames received, skipping file creation");
                return;
            }
        };

        let size_arg = format!("{}x{}", first_frame.width, first_frame.height);
        let video_path_str = match video_path.to_str() {
            Some(s) => s.to_string(),
            None => {
                eprintln!("Invalid video path");
                return;
            }
        };

        let mut child = match std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "rawvideo",
                "-pixel_format", "rgb24",
                "-video_size", &size_arg,
                "-framerate", "30",
                "-i", "pipe:0",
                "-c:v", "libx264",
                "-preset", "fast",
                "-pix_fmt", "yuv420p",
                &video_path_str,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to start ffmpeg (is it installed?): {}", e);
                return;
            }
        };

        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                eprintln!("Failed to get ffmpeg stdin");
                let _ = child.wait();
                return;
            }
        };

        let mut write_ok = stdin.write_all(&first_frame.data).is_ok();
        if write_ok {
            while let Ok(frame) = rx.recv() {
                if stdin.write_all(&frame.data).is_err() {
                    write_ok = false;
                    break;
                }
            }
        }

        if !write_ok {
            eprintln!("Video write error — output may be incomplete");
        }

        drop(stdin); // signals EOF to ffmpeg
        match child.wait() {
            Ok(status) if status.success() => println!("Video saved to {:?}", video_path),
            Ok(status) => eprintln!("ffmpeg exited with non-zero status: {}", status),
            Err(e) => eprintln!("Failed to wait for ffmpeg: {}", e),
        }
    });

    tx
}

fn main() -> Result<(), slint::PlatformError> {
    #[allow(unused_variables)]
    let (menu, quit_item_id) = create_menu();

    #[cfg(target_os = "macos")]
    {
        menu.init_for_nsapp();
        println!("Native menu bar initialized for macOS");
    }

    #[cfg(target_os = "linux")]
    {
        eprintln!("Warning: Native menu bar integration on Linux is limited.");
        eprintln!("Using in-window menu bar from app.slint instead.");
    }

    start_menu_event_handler(quit_item_id);

    // Shared state: current active recording (None when idle).
    let recording_state: Arc<Mutex<Option<Recording>>> = Arc::new(Mutex::new(None));
    // Audio device info, set once the audio thread initializes.
    let audio_info: Arc<Mutex<Option<AudioInfo>>> = Arc::new(Mutex::new(None));

    let app = AppWindow::new()?;
    app.window().set_maximized(true);

    app.on_mood_voted(|mood_id| {
        println!("Mood selected: {} (ID: {})", mood_name(mood_id), mood_id);
    });

    // --- Recording callbacks ---

    let recording_state_start = recording_state.clone();
    let audio_info_start = audio_info.clone();
    let app_weak_start = app.as_weak();
    app.on_start_recording(move || {
        let app = app_weak_start.unwrap();

        {
            let state = recording_state_start.lock().unwrap();
            if state.is_some() {
                println!("Already recording — ignoring duplicate start");
                return;
            }
        }

        let video_enabled = app.get_video_enabled();
        let mic_enabled = app.get_microphone_enabled();
        let mood_id = app.get_selected_mood_id();

        let timestamp = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let dir = std::path::PathBuf::from(format!("./recordings/{}", timestamp));

        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("Failed to create recording directory {:?}: {}", dir, e);
            return;
        }
        println!("Recording started — saving to {:?}", dir);

        write_session_json(&dir, mood_id, &timestamp);

        let audio_tx = if mic_enabled {
            let info = audio_info_start.lock().unwrap();
            if let Some(ref i) = *info {
                Some(spawn_audio_writer(&dir, i.sample_rate, i.channels))
            } else {
                eprintln!("Audio device not yet initialized — skipping audio recording");
                None
            }
        } else {
            None
        };

        let video_tx = if video_enabled {
            Some(spawn_video_writer(&dir))
        } else {
            None
        };

        {
            let mut state = recording_state_start.lock().unwrap();
            *state = Some(Recording { dir, audio_tx, video_tx });
        }

        app.set_is_recording(true);
    });

    let recording_state_stop = recording_state.clone();
    let app_weak_stop = app.as_weak();
    app.on_stop_recording(move || {
        let app = app_weak_stop.unwrap();
        println!("Stopping recording");

        {
            let mut state = recording_state_stop.lock().unwrap();
            *state = None; // drops channels → writer threads finalize files
        }

        app.set_is_recording(false);
    });

    // --- Audio capture thread (preview + recording) ---

    let recording_state_audio = recording_state.clone();
    let audio_info_audio = audio_info.clone();
    let app_weak_audio = app.as_weak();

    std::thread::spawn(move || {
        println!("Audio thread started");

        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                eprintln!("No default audio input device");
                return;
            }
        };

        let config = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to get audio config: {}", e);
                return;
            }
        };

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        println!("Audio config: {} Hz, {} channels, {:?}", sample_rate, channels, config.sample_format());

        {
            let mut info = audio_info_audio.lock().unwrap();
            *info = Some(AudioInfo { sample_rate, channels });
        }

        let buffer_size = sample_rate as usize * 3;
        let audio_buffer = Arc::new(Mutex::new(Vec::<f32>::with_capacity(buffer_size)));

        // Shared closure: update preview buffer + send to recording channel.
        // Captured Arcs are all Clone, so the closure is Clone.
        let audio_buffer_inner = audio_buffer.clone();
        let rec_state_inner = recording_state_audio.clone();
        let process = move |samples_f32: Vec<f32>| {
            // Preview buffer (circular, last 3 s)
            {
                let mut buf = audio_buffer_inner.lock().unwrap();
                let max_len = buf.capacity();
                buf.extend_from_slice(&samples_f32);
                if buf.len() > max_len {
                    let excess = buf.len() - max_len;
                    buf.drain(0..excess);
                }
            }
            // Recording
            let audio_tx = {
                let rec = rec_state_inner.lock().unwrap();
                rec.as_ref().and_then(|r| r.audio_tx.clone())
            };
            if let Some(tx) = audio_tx {
                let _ = tx.try_send(samples_f32);
            }
        };

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let stream_config = config.into();
                let p = process.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| p(data.to_vec()),
                    |e| eprintln!("Audio stream error: {}", e),
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let stream_config = config.into();
                let p = process.clone();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        p(data.iter().map(|&s| s as f32 / 32768.0).collect())
                    },
                    |e| eprintln!("Audio stream error: {}", e),
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let stream_config = config.into();
                let p = process;
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        p(data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect())
                    },
                    |e| eprintln!("Audio stream error: {}", e),
                    None,
                )
            }
            fmt => {
                eprintln!("Unsupported audio sample format: {:?}", fmt);
                return;
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build audio input stream: {}", e);
                return;
            }
        };

        if let Err(e) = stream.play() {
            eprintln!("Failed to start audio stream: {}", e);
            return;
        }
        println!("Audio stream started");

        // Preview update loop: ~20 FPS.
        let sample_rate_usize = sample_rate as usize;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));

            let envelope = {
                let buf = audio_buffer.lock().unwrap();
                let target_samples = sample_rate_usize * 3;
                let step = (target_samples / 160).max(1);
                (0..160)
                    .map(|i| {
                        let steps_from_end = 159 - i;
                        let end_idx = buf.len().saturating_sub(steps_from_end * step);
                        let start_idx = buf.len().saturating_sub((steps_from_end + 1) * step);
                        if start_idx >= end_idx {
                            0.0
                        } else {
                            let window = &buf[start_idx..end_idx];
                            (window.iter().map(|&s| s * s).sum::<f32>() / window.len() as f32).sqrt()
                        }
                    })
                    .collect::<Vec<f32>>()
            };

            let app_weak_clone = app_weak_audio.clone();
            slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak_clone.upgrade() {
                    let model = slint::ModelRc::new(slint::VecModel::from(envelope));
                    app.set_audio_samples(model);
                }
            }).ok();
        }
    });

    // --- Camera capture thread (preview + recording) ---

    let recording_state_video = recording_state.clone();
    let app_weak_cam = app.as_weak();
    let timer = slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        let recording_state_thread = recording_state_video.clone();
        let app_weak_for_thread = app_weak_cam.clone();
        std::thread::spawn(move || {
            println!("Camera thread started");

            let camera_result = Camera::new(
                CameraIndex::Index(0),
                RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate),
            );

            let mut camera = match camera_result {
                Ok(cam) => {
                    println!("Webcam initialized successfully");
                    cam
                }
                Err(e) => {
                    eprintln!("Failed to initialize webcam: {}", e);
                    let app_weak_clone = app_weak_for_thread.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak_clone.upgrade() {
                            app.set_webcam_preview(create_no_camera_image(160, 90));
                        }
                    }).ok();
                    return;
                }
            };

            if let Err(e) = camera.open_stream() {
                eprintln!("Failed to open camera stream: {}", e);
                let app_weak_clone = app_weak_for_thread.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_clone.upgrade() {
                        app.set_webcam_preview(create_no_camera_image(160, 90));
                    }
                }).ok();
                return;
            }

            println!("Camera stream opened, starting capture loop");

            let ui_processing = Arc::new(AtomicBool::new(false));
            let mut frame_counter: u64 = 0;
            let mut frames_skipped: u64 = 0;

            loop {
                match camera.frame() {
                    Ok(frame) => {
                        frame_counter += 1;

                        // Quickly check if we need this frame for recording.
                        let video_tx = {
                            let rec = recording_state_thread.lock().unwrap();
                            rec.as_ref().and_then(|r| r.video_tx.clone())
                        };
                        let needs_ui = frame_counter % 2 == 0;
                        let needs_recording = video_tx.is_some();

                        if !needs_ui && !needs_recording {
                            continue; // skip decode entirely
                        }

                        if needs_ui && ui_processing.load(Ordering::Relaxed) {
                            frames_skipped += 1;
                            if frames_skipped % 10 == 0 {
                                println!("Skipped {} UI frames (event loop busy)", frames_skipped);
                            }
                            if !needs_recording {
                                continue;
                            }
                        }

                        let decoded = match frame.decode_image::<RgbFormat>() {
                            Ok(img) => img,
                            Err(e) => {
                                eprintln!("Failed to decode camera frame: {}", e);
                                continue;
                            }
                        };

                        let width = decoded.width();
                        let height = decoded.height();
                        let pixel_data = decoded.into_raw();

                        // Send to recording writer.
                        if let Some(tx) = video_tx {
                            let _ = tx.try_send(RawFrame {
                                data: pixel_data.clone(),
                                width,
                                height,
                            });
                        }

                        // Update UI preview on even frames.
                        if needs_ui && !ui_processing.load(Ordering::Relaxed) {
                            ui_processing.store(true, Ordering::Relaxed);
                            let app_weak_clone = app_weak_for_thread.clone();
                            let ui_processing_clone = ui_processing.clone();
                            if let Err(e) = slint::invoke_from_event_loop(move || {
                                if let Some(app) = app_weak_clone.upgrade() {
                                    let buf = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                                        &pixel_data, width, height,
                                    );
                                    app.set_webcam_preview(Image::from_rgb8(buf));
                                }
                                ui_processing_clone.store(false, Ordering::Relaxed);
                            }) {
                                eprintln!("Failed to invoke from event loop: {:?}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to capture frame: {}", e);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
            println!("Camera thread exiting");
        });
    });

    let _timer = timer;
    app.run()
}
