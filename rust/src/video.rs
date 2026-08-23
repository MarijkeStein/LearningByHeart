use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::recording::VideoSink;
use crate::AppWindow;


/// Spawns a background thread that continuously captures frames from the default
/// webcam and pushes them to the UI via Slint's event loop.
///
/// `capture_active` controls whether the thread actually grabs frames; when false
/// the thread sleeps cheaply, allowing the sensor to stay warm for a fast resume.
/// The thread exits automatically once the Slint event loop is gone.
pub fn start_webcam_preview(
    app_weak: slint::Weak<AppWindow>,
    capture_active: Arc<AtomicBool>,
    recording_sink: VideoSink,
) {
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

                            // Non-blocking send to recording writer.
                            if let Ok(guard) = recording_sink.try_lock() {
                                if let Some(tx) = guard.as_ref() {
                                    let _ = tx.try_send((width, height, raw.clone()));
                                }
                            }

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