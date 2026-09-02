use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use uuid::Uuid;

use crate::recording::{HrRecord, HrSink};
use crate::AppWindow;

// Hardcoded target device MAC address (Linux/BlueZ peripheral ID format)
const DEVICE_MAC: &str = "F0:13:C3:EE:F2:A8";

// Bluetooth SIG GATT UUIDs for heart rate
const HEART_RATE_SERVICE_UUID: Uuid =
    Uuid::from_u128(0x0000_180d_0000_1000_8000_0080_5f9b_34fb);
const HEART_RATE_MEASUREMENT_UUID: Uuid =
    Uuid::from_u128(0x0000_2a37_0000_1000_8000_0080_5f9b_34fb);

const SCAN_SECS: u64 = 2;
const RETRY_SECS: u64 = 2;

struct HrMeasurement {
    bpm: u16,
    // RR intervals in milliseconds (converted from 1/1024 s GATT units)
    rr_intervals_ms: Vec<u16>,
}

fn parse_measurement(data: &[u8]) -> Option<HrMeasurement> {
    if data.is_empty() {
        return None;
    }

    let flags = data[0];
    let hr_is_u16 = flags & 0x01 != 0;
    let energy_present = flags & 0x08 != 0;
    let rr_present = flags & 0x10 != 0;

    let mut offset = 1usize;

    let bpm = if hr_is_u16 {
        if data.len() < offset + 2 {
            return None;
        }
        let v = u16::from_le_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        v
    } else {
        if data.len() < offset + 1 {
            return None;
        }
        let v = data[offset] as u16;
        offset += 1;
        v
    };

    if energy_present {
        offset += 2; // skip Energy Expended field
    }

    let mut rr_intervals_ms = Vec::new();
    if rr_present {
        while offset + 1 < data.len() {
            let raw = u16::from_le_bytes([data[offset], data[offset + 1]]);
            // GATT unit is 1/1024 s; convert to ms
            rr_intervals_ms.push((raw as u32 * 1000 / 1024) as u16);
            offset += 2;
        }
    }

    Some(HrMeasurement { bpm, rr_intervals_ms })
}

/// Spawns a background thread that scans for a Bluetooth heart rate sensor,
/// connects to it, and streams HR measurements to the UI. Automatically
/// reconnects if the sensor disconnects.
pub fn start_heartbeat_monitor(app_weak: slint::Weak<AppWindow>, hr_sink: HrSink) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Bluetooth: failed to create async runtime: {e}");
                return;
            }
        };
        rt.block_on(run_monitor(app_weak, hr_sink));
    });
}

fn push_display(app_weak: &slint::Weak<AppWindow>, text: &str) -> bool {
    let owned = text.to_owned();
    slint::invoke_from_event_loop({
        let weak = app_weak.clone();
        move || {
            if let Some(app) = weak.upgrade() {
                app.set_heart_rate_display(owned.into());
            }
        }
    })
    .is_ok()
}

fn push_sensor_found(app_weak: &slint::Weak<AppWindow>, found: bool) -> bool {
    slint::invoke_from_event_loop({
        let weak = app_weak.clone();
        move || {
            if let Some(app) = weak.upgrade() {
                app.set_heartbeat_sensor_found(found);
                if found {
                    app.set_heartbeat_enabled(true);
                }
            }
        }
    })
    .is_ok()
}

fn push_pulse(app_weak: &slint::Weak<AppWindow>) {
    let weak = app_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            app.set_heart_beat_pulse(true);
        }
    });
    let weak = app_weak.clone();
    tokio::spawn(async move {
        // Hold long enough for the font-size animation to finish growing before
        // it starts shrinking back; 200 ms keeps each bump crisp and separate.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_heart_beat_pulse(false);
            }
        });
    });
}

/// Schedule one visual pulse per R-R interval so the heart bumps in sync with
/// each actual heartbeat. Falls back to one pulse per notification if the
/// sensor does not report R-R data.
fn push_rr_pulses(app_weak: &slint::Weak<AppWindow>, rr_intervals_ms: &[u16]) {
    if rr_intervals_ms.is_empty() {
        // No RR intervals means no detected heartbeat — keep the heart still.
        return;
    }
    let mut offset_ms: u64 = 0;
    for &rr in rr_intervals_ms {
        if offset_ms == 0 {
            push_pulse(app_weak);
        } else {
            let weak = app_weak.clone();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(offset_ms)).await;
                push_pulse(&weak);
            });
        }
        offset_ms += rr as u64;
    }
}

async fn find_hr_peripheral(adapter: &Adapter) -> Option<Peripheral> {
    let filter = ScanFilter { services: vec![HEART_RATE_SERVICE_UUID] };
    if let Err(e) = adapter.start_scan(filter).await {
        eprintln!("BLE scan error: {e}");
        return None;
    }

    tokio::time::sleep(tokio::time::Duration::from_secs(SCAN_SECS)).await;
    let _ = adapter.stop_scan().await;

    let peripherals = match adapter.peripherals().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("BLE peripherals error: {e}");
            return None;
        }
    };

    let mut target = None;
    for p in peripherals {
        let props = p.properties().await.ok().flatten();
        let services = props.as_ref().map(|pr| pr.services.as_slice()).unwrap_or(&[]);
        if !services.contains(&HEART_RATE_SERVICE_UUID) {
            continue;
        }
        let name = props
            .as_ref()
            .and_then(|pr| pr.local_name.as_deref())
            .unwrap_or("<unnamed>");
        let id = p.id().to_string();
        // BlueZ peripheral IDs look like "hci0/dev_F0_13_C3_EE_F2_A8"
        let mac_fragment = DEVICE_MAC.replace(':', "_");
        let is_target = id.to_ascii_uppercase().contains(&mac_fragment.to_ascii_uppercase());
        eprintln!("  HR device: {name} [{id}]{}", if is_target { " ← target" } else { "" });
        if is_target {
            target = Some(p);
        }
    }

    target
}

async fn run_monitor(app_weak: slint::Weak<AppWindow>, hr_sink: HrSink) {
    let manager = match Manager::new().await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Bluetooth unavailable: {e}");
            push_display(&app_weak, "Bluetooth not available");
            return;
        }
    };

    let adapters = match manager.adapters().await {
        Ok(a) if !a.is_empty() => a,
        Ok(_) => {
            eprintln!("No Bluetooth adapter found");
            push_display(&app_weak, "No Bluetooth adapter");
            return;
        }
        Err(e) => {
            eprintln!("Bluetooth adapter error: {e}");
            push_display(&app_weak, "Bluetooth error");
            return;
        }
    };

    let adapter = adapters.into_iter().next().unwrap();

    loop {
        if !push_display(&app_weak, "Scanning for HR sensor...") {
            return;
        }
        push_sensor_found(&app_weak, false);
        eprintln!("Scanning for HR sensor {DEVICE_MAC} ({SCAN_SECS}s)...");

        let peripheral = match find_hr_peripheral(&adapter).await {
            Some(p) => p,
            None => {
                eprintln!("No HR sensor found; retrying in {RETRY_SECS}s");
                tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
                continue;
            }
        };

        if let Err(e) = peripheral.connect().await {
            eprintln!("HR sensor connect failed: {e}");
            tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
            continue;
        }

        if let Err(e) = peripheral.discover_services().await {
            eprintln!("BLE service discovery failed: {e}");
            let _ = peripheral.disconnect().await;
            tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
            continue;
        }

        let chars = peripheral.characteristics();
        let hr_char = match chars.iter().find(|c| c.uuid == HEART_RATE_MEASUREMENT_UUID) {
            Some(c) => c.clone(),
            None => {
                eprintln!("HR measurement characteristic not found on device");
                let _ = peripheral.disconnect().await;
                tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
                continue;
            }
        };

        if let Err(e) = peripheral.subscribe(&hr_char).await {
            eprintln!("HR notification subscribe failed: {e}");
            let _ = peripheral.disconnect().await;
            tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
            continue;
        }

        eprintln!("HR sensor connected, receiving data");
        push_sensor_found(&app_weak, true);

        let mut notifs = match peripheral.notifications().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("HR notification stream error: {e}");
                let _ = peripheral.disconnect().await;
                tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
                continue;
            }
        };

        while let Some(notif) = notifs.next().await {
            if notif.uuid != HEART_RATE_MEASUREMENT_UUID {
                continue;
            }

            let Some(m) = parse_measurement(&notif.value) else {
                continue;
            };

            if !m.rr_intervals_ms.is_empty() {
                eprintln!("HR: {} BPM  RR: {:?} ms", m.bpm, m.rr_intervals_ms);
            } else {
                eprintln!("HR: {} BPM", m.bpm);
            }

            // Forward to recording writer if active.
            if let Ok(guard) = hr_sink.try_lock() {
                if let Some(tx) = guard.as_ref() {
                    let record = HrRecord {
                        timestamp_ms: chrono::Local::now().timestamp_millis(),
                        bpm: m.bpm,
                        rr_intervals_ms: m.rr_intervals_ms.clone(),
                    };
                    let _ = tx.try_send(record);
                }
            }

            let display = format!("{} BPM", m.bpm);
            if !push_display(&app_weak, &display) {
                return; // event loop gone — app is closing
            }
            push_rr_pulses(&app_weak, &m.rr_intervals_ms);
        }

        eprintln!("HR sensor disconnected; reconnecting in {RETRY_SECS}s");
        let _ = peripheral.disconnect().await;
        tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_SECS)).await;
    }
}
