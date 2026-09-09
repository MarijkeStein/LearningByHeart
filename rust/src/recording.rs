use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

// ── Public types shared with capture modules ────────────────────────────────

pub type VideoFrame = (u32, u32, Vec<u8>); // (width, height, rgb24)
pub type VideoSink  = Arc<Mutex<Option<mpsc::SyncSender<VideoFrame>>>>;
pub type AudioSink  = Arc<Mutex<Option<mpsc::SyncSender<Vec<f32>>>>>;

pub struct HrRecord {
    pub timestamp_ms: i64,
    pub bpm: u16,
    pub rr_intervals_ms: Vec<u16>,
}
pub type HrSink = Arc<Mutex<Option<mpsc::SyncSender<HrRecord>>>>;

// ── Active recording handle ─────────────────────────────────────────────────

/// Holds one sender end for each active writer thread.
/// Dropping this (via `stop_recording`) closes the channels and lets the
/// writer threads finalise their files.
pub struct ActiveRecording {
    pub dir: PathBuf,
    _video_tx: Option<mpsc::SyncSender<VideoFrame>>,
    _audio_tx: Option<mpsc::SyncSender<Vec<f32>>>,
    _hr_tx:    Option<mpsc::SyncSender<HrRecord>>,
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn start_recording(
    dir:               PathBuf,
    video_sink:        &VideoSink,
    audio_sink:        &AudioSink,
    audio_sample_rate: u32,
    audio_channels:    u16,
    hr_sink:           &HrSink,
    video_enabled:     bool,
    audio_enabled:     bool,
) -> std::io::Result<ActiveRecording> {
    std::fs::create_dir_all(&dir)?;

    let mut rec = ActiveRecording {
        dir: dir.clone(),
        _video_tx: None,
        _audio_tx: None,
        _hr_tx:    None,
    };

    if video_enabled {
        let (tx, rx) = mpsc::sync_channel::<VideoFrame>(256);
        let path = dir.join("video.mp4");
        thread::spawn(move || write_video(rx, path));
        *video_sink.lock().unwrap() = Some(tx.clone());
        rec._video_tx = Some(tx);
    }

    if audio_enabled {
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(512);
        let path = dir.join("audio.wav");
        thread::spawn(move || write_audio(rx, path, audio_sample_rate, audio_channels));
        *audio_sink.lock().unwrap() = Some(tx.clone());
        rec._audio_tx = Some(tx);
    }

    // Always set up the HR writer so data is captured even when the sensor
    // connects after recording has already started.
    {
        let (tx, rx) = mpsc::sync_channel::<HrRecord>(1024);
        let path = dir.join("heartbeat.json");
        thread::spawn(move || write_heartbeat(rx, path));
        *hr_sink.lock().unwrap() = Some(tx.clone());
        rec._hr_tx = Some(tx);
    }

    Ok(rec)
}

/// Removes the shared-sink senders (stopping new data flowing in), then drops
/// the private sender copies in `rec`, closing the channels and letting writer
/// threads flush and finalise their output files.
pub fn stop_recording(
    rec:        ActiveRecording,
    video_sink: &VideoSink,
    audio_sink: &AudioSink,
    hr_sink:    &HrSink,
) {
    *video_sink.lock().unwrap() = None;
    *audio_sink.lock().unwrap() = None;
    *hr_sink.lock().unwrap()    = None;
    drop(rec);
}

// ── Writer threads ──────────────────────────────────────────────────────────

fn write_video(rx: mpsc::Receiver<VideoFrame>, path: PathBuf) {
    // Wait for the first frame to learn the actual capture resolution.
    let first = match rx.recv() {
        Ok(f) => f,
        Err(_) => return,
    };
    let (width, height, _) = &first;
    let size_arg = format!("{}x{}", width, height);

    let mut child = match Command::new("ffmpeg")
        .args([
            "-y",
            "-f",            "rawvideo",
            "-pixel_format", "rgb24",
            "-video_size",   &size_arg,
            "-framerate",    "15",
            "-i",            "pipe:0",
            "-c:v",          "libx264",
            "-preset",       "fast",
            "-crf",          "23",
            "-pix_fmt",      "yuv420p",
            path.to_str().unwrap_or("video.mp4"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Video recording: failed to start ffmpeg: {e}");
            return;
        }
    };

    let mut stdin = match child.stdin.take() {
        Some(s) => s,
        None => {
            eprintln!("Video recording: could not get ffmpeg stdin");
            let _ = child.wait();
            return;
        }
    };

    // Write first frame, then stream all subsequent frames.
    let frames = std::iter::once(first).chain(rx);
    for (_, _, rgb) in frames {
        if let Err(e) = stdin.write_all(&rgb) {
            eprintln!("Video recording: pipe write error: {e}");
            break;
        }
    }

    drop(stdin); // close pipe → ffmpeg finishes encoding
    if let Err(e) = child.wait() {
        eprintln!("Video recording: ffmpeg wait error: {e}");
    }
}

fn write_audio(rx: mpsc::Receiver<Vec<f32>>, path: PathBuf, sample_rate: u32, channels: u16) {
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer = match hound::WavWriter::create(&path, spec) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Audio recording: failed to create WAV: {e}");
            return;
        }
    };

    let mut samples_written: u64 = 0;
    for chunk in rx {
        for sample in chunk {
            if let Err(e) = writer.write_sample(sample) {
                eprintln!("Audio recording: write error: {e}");
                return;
            }
            samples_written += 1;
        }
    }

    if let Err(e) = writer.finalize() {
        eprintln!("Audio recording: failed to finalise WAV: {e}");
    }

    if samples_written == 0 {
        let _ = std::fs::remove_file(&path);
    }
}

fn write_heartbeat(rx: mpsc::Receiver<HrRecord>, path: PathBuf) {
    let records: Vec<HrRecord> = rx.into_iter().collect();

    if records.is_empty() {
        return;
    }

    let mut json = String::from("[\n");
    for (i, r) in records.iter().enumerate() {
        let rr: Vec<String> = r.rr_intervals_ms.iter().map(|v| v.to_string()).collect();
        let comma = if i + 1 < records.len() { "," } else { "" };
        json.push_str(&format!(
            "  {{\"timestamp_ms\": {}, \"bpm\": {}, \"rr_intervals_ms\": [{}]}}{}\n",
            r.timestamp_ms,
            r.bpm,
            rr.join(", "),
            comma,
        ));
    }
    json.push_str("]\n");

    if let Err(e) = std::fs::write(&path, json) {
        eprintln!("Heartbeat recording: failed to write JSON: {e}");
    }
}
