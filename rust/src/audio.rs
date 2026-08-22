use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use slint::{ModelRc, VecModel};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::AppWindow;

/// Generic stream builder — converts any `SizedSample` format to f32 in the callback.
/// Stores per-frame peak (max absolute value across all channels) into `pending`.
fn build_audio_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    pending: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        *config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mut buf = pending.lock().unwrap();
            for frame in data.chunks(channels.max(1)) {
                let peak = frame.iter()
                    .map(|&s| f32::from_sample_(s).abs())
                    .fold(0.0f32, f32::max);
                buf.push(peak);
            }
            // Prevent unbounded growth if the display thread falls behind.
            if buf.len() > 8192 {
                let keep_from = buf.len() - 8192;
                buf.drain(..keep_from);
            }
        },
        |e| eprintln!("Audio stream error: {e}"),
        None,
    )
}

/// Spawns a background thread that captures microphone input and pushes a
/// scrolling 160-slot peak waveform to the UI at ~15 fps.
///
/// The audio callback runs at hardware rate (~375×/sec) but only stores peaks
/// into a shared buffer. The display thread wakes up at 15 Hz, drains that
/// buffer in one lock, and calls `invoke_from_event_loop` exactly once per frame.
///
/// `capture_active` mirrors the microphone-enabled checkbox. When false the
/// stream is paused (hardware stays idle) and the waveform is cleared.
pub fn start_audio_preview(app_weak: slint::Weak<AppWindow>, capture_active: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let host = cpal::default_host();

        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                eprintln!("No audio input device found");
                let _ = slint::invoke_from_event_loop({
                    let weak = app_weak.clone();
                    move || { if let Some(app) = weak.upgrade() { app.set_microphone_enabled(false); } }
                });
                return;
            }
        };

        let supported = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("No default audio config: {e}");
                let _ = slint::invoke_from_event_loop({
                    let weak = app_weak.clone();
                    move || { if let Some(app) = weak.upgrade() { app.set_microphone_enabled(false); } }
                });
                return;
            }
        };

        let channels = supported.channels() as usize;
        let config: cpal::StreamConfig = supported.clone().into();
        let pending: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(4096)));

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_audio_input_stream::<f32>(&device, &config, channels, pending.clone()),
            SampleFormat::I16 => build_audio_input_stream::<i16>(&device, &config, channels, pending.clone()),
            SampleFormat::I32 => build_audio_input_stream::<i32>(&device, &config, channels, pending.clone()),
            SampleFormat::U16 => build_audio_input_stream::<u16>(&device, &config, channels, pending.clone()),
            SampleFormat::U32 => build_audio_input_stream::<u32>(&device, &config, channels, pending.clone()),
            SampleFormat::F64 => build_audio_input_stream::<f64>(&device, &config, channels, pending.clone()),
            fmt => {
                eprintln!("Unsupported audio sample format: {fmt:?}");
                let _ = slint::invoke_from_event_loop({
                    let weak = app_weak.clone();
                    move || { if let Some(app) = weak.upgrade() { app.set_microphone_enabled(false); } }
                });
                return;
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build audio stream: {e}");
                let _ = slint::invoke_from_event_loop({
                    let weak = app_weak.clone();
                    move || { if let Some(app) = weak.upgrade() { app.set_microphone_enabled(false); } }
                });
                return;
            }
        };

        if let Err(e) = stream.play() {
            eprintln!("Failed to start audio stream: {e}");
            let _ = slint::invoke_from_event_loop({
                let weak = app_weak.clone();
                move || { if let Some(app) = weak.upgrade() { app.set_microphone_enabled(false); } }
            });
            return;
        }

        // Scrolling display buffer: 160 display values, one per waveform bar.
        let mut display_buf: VecDeque<f32> = VecDeque::with_capacity(160);
        // Noise floor estimator: running minimum over the last ~10 s (150 ticks).
        // Pre-filled with 1.0 so the first real peak immediately becomes the floor.
        let mut floor_buf: VecDeque<f32> = VecDeque::from(vec![1.0f32; 150]);
        let mut prev_active = capture_active.load(Ordering::Relaxed);
        let display_interval = std::time::Duration::from_millis(67); // ~15 fps
        let mut debug_tick = 0u32;

        loop {
            std::thread::sleep(display_interval);

            let active = capture_active.load(Ordering::Relaxed);

            // Handle enable/disable transitions without tearing down the stream.
            if active != prev_active {
                prev_active = active;
                if active {
                    pending.lock().unwrap().clear(); // discard stale samples
                    let _ = stream.play();
                } else {
                    let _ = stream.pause();
                    display_buf.clear();
                    // Clear the waveform in the UI.
                    let result = slint::invoke_from_event_loop({
                        let weak = app_weak.clone();
                        move || {
                            if let Some(app) = weak.upgrade() {
                                app.set_audio_samples(ModelRc::new(VecModel::from(vec![])));
                            }
                        }
                    });
                    if result.is_err() { break; }
                }
            }

            if !active {
                continue;
            }

            // Drain all samples collected since last tick; compute peak for this window.
            // Returns None if the buffer was empty (stream just resumed, no audio yet).
            let window_peak: Option<f32> = {
                let mut buf = pending.lock().unwrap();
                if buf.is_empty() {
                    None
                } else {
                    let peak = buf.iter().copied().fold(0.0f32, f32::max);
                    buf.clear();
                    Some(peak)
                }
            };

            // Skip this tick if no audio arrived yet — avoids pushing 0.0 into
            // floor_buf right after stream resume, which would corrupt the floor.
            let Some(window_peak) = window_peak else { continue };

            // Update noise floor: running minimum over the last 150 ticks (~10 s).
            floor_buf.push_back(window_peak);
            floor_buf.pop_front();
            let noise_floor = floor_buf.iter().copied().fold(f32::MAX, f32::min);

            // Subtract the noise floor so that ambient mic noise reads as zero.
            // Scale up by 5× so modest speech above the floor fills the display,
            // then apply Michaelis-Menten soft-knee (k=0.2) to avoid hard clipping.
            let above_floor = (window_peak - noise_floor).max(0.0) * 5.0;
            let display_val = above_floor / (above_floor + 0.2);

            // Periodic diagnostic.
            debug_tick += 1;
            if debug_tick % 45 == 0 {
                eprintln!("Audio raw={window_peak:.4}  floor={noise_floor:.4}  above={above_floor:.4}  display={display_val:.4}");
            }

            // Scroll: push compressed value, drop oldest when full.
            if display_buf.len() >= 160 {
                display_buf.pop_front();
            }
            display_buf.push_back(display_val);

            let samples: Vec<f32> = display_buf.iter().copied().collect();
            let result = slint::invoke_from_event_loop({
                let weak = app_weak.clone();
                move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_audio_samples(ModelRc::new(VecModel::from(samples)));
                    }
                }
            });
            if result.is_err() { break; } // Event loop gone — app is closing.
        }
        // `stream` is dropped here, which stops capture.
    });
}