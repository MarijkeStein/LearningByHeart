slint::include_modules!();
use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use slint::{Image, SharedPixelBuffer, Rgb8Pixel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Mutex;

// Creates a crossed-out rectangle placeholder image for when camera is unavailable
fn create_no_camera_image(width: u32, height: u32) -> Image {
    let mut pixel_data = vec![0u8; (width * height * 3) as usize];

    // Fill with dark gray background
    for chunk in pixel_data.chunks_mut(3) {
        chunk[0] = 60;  // R
        chunk[1] = 60;  // G
        chunk[2] = 60;  // B
    }

    let border_width = 3;
    let cross_width = 4;

    // Draw border rectangle
    for y in 0..height {
        for x in 0..width {
            let is_border = x < border_width || x >= width - border_width ||
                          y < border_width || y >= height - border_width;

            if is_border {
                let idx = ((y * width + x) * 3) as usize;
                pixel_data[idx] = 180;      // R
                pixel_data[idx + 1] = 180;  // G
                pixel_data[idx + 2] = 180;  // B
            }
        }
    }

    // Draw diagonal cross (X)
    for y in 0..height {
        for x in 0..width {
            let diag1 = (x as i32 - y as i32).abs() < cross_width as i32;
            let diag2 = (x as i32 - (height as i32 - 1 - y as i32)).abs() < cross_width as i32;

            if diag1 || diag2 {
                let idx = ((y * width + x) * 3) as usize;
                pixel_data[idx] = 200;      // R
                pixel_data[idx + 1] = 60;   // G (darker for red-ish cross)
                pixel_data[idx + 2] = 60;   // B
            }
        }
    }

    let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&pixel_data, width, height);
    Image::from_rgb8(buffer)
}

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

    // Connect recording callbacks
    let app_weak = app.as_weak();
    app.on_start_recording(move || {
        let app = app_weak.unwrap();
        let video = app.get_video_enabled();
        let mic = app.get_microphone_enabled();
        let heartbeat = app.get_heartbeat_enabled();

        println!("Starting recording with:");
        println!("  - Video: {}", video);
        println!("  - Microphone: {}", mic);
        println!("  - Heartbeat: {}", heartbeat);

        app.set_is_recording(true);

        // TODO: Initialize sensor streams based on enabled flags
    });

    let app_weak = app.as_weak();
    app.on_stop_recording(move || {
        let app = app_weak.unwrap();

        println!("Stopping recording");

        app.set_is_recording(false);

        // TODO: Stop sensor streams and save recorded data
    });

    // Initialize microphone audio capture
    let app_weak_audio = app.as_weak();
    std::thread::spawn(move || {
        println!("Audio thread started");

        // Try to get the default audio input device
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(dev) => {
                println!("Microphone initialized: {}", dev.name().unwrap_or_else(|_| "Unknown".to_string()));
                dev
            }
            None => {
                eprintln!("Failed to get default input device");
                return;
            }
        };

        // Get the default input config
        let config = match device.default_input_config() {
            Ok(cfg) => {
                println!("Audio config: {} Hz, {} channels, {:?}",
                    cfg.sample_rate().0,
                    cfg.channels(),
                    cfg.sample_format()
                );
                cfg
            }
            Err(e) => {
                eprintln!("Failed to get default input config: {}", e);
                return;
            }
        };

        // Buffer to store recent audio samples for visualization
        // Store last 1024 samples for waveform display
        let audio_buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let audio_buffer_clone = audio_buffer.clone();

        // Build the audio input stream
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let stream_config = config.into();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        let mut buffer = audio_buffer_clone.lock().unwrap();
                        buffer.extend_from_slice(data);
                        // Keep only the last 1024 samples
                        if buffer.len() > 1024 {
                            let len = buffer.len();
                            buffer.drain(0..len - 1024);
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let stream_config = config.into();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let mut buffer = audio_buffer_clone.lock().unwrap();
                        // Convert i16 to f32 (normalize to -1.0 to 1.0)
                        buffer.extend(data.iter().map(|&s| s as f32 / 32768.0));
                        if buffer.len() > 1024 {
                            let len = buffer.len();
                            buffer.drain(0..len - 1024);
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let stream_config = config.into();
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        let mut buffer = audio_buffer_clone.lock().unwrap();
                        // Convert u16 to f32 (normalize to -1.0 to 1.0)
                        buffer.extend(data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0));
                        if buffer.len() > 1024 {
                            let len = buffer.len();
                            buffer.drain(0..len - 1024);
                        }
                    },
                    |err| eprintln!("Audio stream error: {}", err),
                    None,
                )
            }
            _ => {
                eprintln!("Unsupported sample format: {:?}", config.sample_format());
                return;
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build input stream: {}", e);
                return;
            }
        };

        // Start the audio stream
        if let Err(e) = stream.play() {
            eprintln!("Failed to start audio stream: {}", e);
            return;
        }

        println!("Audio stream started");

        // Keep the stream alive and periodically update UI with audio samples
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50)); // Update at ~20 FPS

            let samples_to_display = {
                let buffer = audio_buffer.lock().unwrap();

                // Downsample to 32 samples for visualization (matching the UI component)
                if buffer.len() >= 32 {
                    let step = buffer.len() / 32;
                    (0..32)
                        .map(|i| {
                            // Get RMS of a window for smoother visualization
                            let start = i * step;
                            let end = ((i + 1) * step).min(buffer.len());
                            let window = &buffer[start..end];
                            let rms = (window.iter().map(|&s| s * s).sum::<f32>() / window.len() as f32).sqrt();
                            rms
                        })
                        .collect::<Vec<f32>>()
                } else {
                    buffer.clone()
                }
            };

            // Update UI on event loop thread
            let app_weak_clone = app_weak_audio.clone();
            let samples_vec = samples_to_display.clone();
            slint::invoke_from_event_loop(move || {
                if let Some(app) = app_weak_clone.upgrade() {
                    let model = slint::ModelRc::new(slint::VecModel::from(samples_vec));
                    app.set_audio_samples(model);
                }
            }).ok();
        }
    });

    // Initialize webcam preview after a short delay to ensure event loop is running
    let app_weak = app.as_weak();
    let timer = slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        let app_weak_for_thread = app_weak.clone();
        std::thread::spawn(move || {
            println!("Camera thread started");

            // Try to open the webcam
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

                    // Display crossed-out rectangle placeholder on UI thread
                    let app_weak_clone = app_weak_for_thread.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_weak_clone.upgrade() {
                            let no_camera_img = create_no_camera_image(160, 90);
                            app.set_webcam_preview(no_camera_img);
                        }
                    }).ok();
                    return;
                }
            };

            // Start the camera stream
            if let Err(e) = camera.open_stream() {
                eprintln!("Failed to open camera stream: {}", e);

                // Display crossed-out rectangle placeholder on UI thread
                let app_weak_clone = app_weak_for_thread.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(app) = app_weak_clone.upgrade() {
                        let no_camera_img = create_no_camera_image(160, 90);
                        app.set_webcam_preview(no_camera_img);
                    }
                }).ok();
                return;
            }

            println!("Camera stream opened, starting capture loop");

            // Use atomic flag to track if UI is still processing previous frame
            let processing = Arc::new(AtomicBool::new(false));

            // Capture frames continuously with minimal latency and frame skipping
            let mut frame_counter = 0;
            let mut frames_skipped = 0;
            loop {
                match camera.frame() {
                    Ok(frame) => {
                        frame_counter += 1;

                        // Display every other frame to reduce UI load and improve responsiveness
                        if frame_counter % 2 != 0 {
                            continue;
                        }

                        // Skip this frame if the UI is still processing the previous one
                        if processing.load(Ordering::Relaxed) {
                            frames_skipped += 1;
                            if frames_skipped % 10 == 0 {
                                println!("Skipped {} frames (UI thread busy)", frames_skipped);
                            }
                            continue;
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

                        // Get raw pixel data
                        let pixel_data = decoded.into_raw();

                        // Mark as processing
                        processing.store(true, Ordering::Relaxed);

                        // Update the UI from event loop thread
                        let app_weak_clone = app_weak_for_thread.clone();
                        let processing_clone = processing.clone();
                        if let Err(e) = slint::invoke_from_event_loop(move || {
                            if let Some(app) = app_weak_clone.upgrade() {
                                let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
                                    &pixel_data,
                                    width,
                                    height,
                                );
                                let image = Image::from_rgb8(buffer);
                                app.set_webcam_preview(image);
                            }
                            // Mark as done processing
                            processing_clone.store(false, Ordering::Relaxed);
                        }) {
                            eprintln!("Failed to invoke from event loop: {:?}", e);
                            break;
                        }

                        // No sleep - capture as fast as possible and let frame skipping handle rate limiting
                    }
                    Err(e) => {
                        eprintln!("Failed to capture frame: {}", e);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                }
            }
            println!("Camera thread exiting");
        });
    });

    // Timer is kept alive by being in scope
    let _timer = timer;

    app.run()
}
