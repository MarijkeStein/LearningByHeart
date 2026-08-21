import asyncio
from bleak import BleakClient, BleakScanner

DEVICE_NAME_PREFIX = "TICKR FIT"

# Standard Bluetooth SIG GATT UUIDs for Heart Rate
HR_MEASUREMENT_UUID = "00002a37-0000-1000-8000-00805f9b34fb"

def parse_heart_rate(sender, data: bytearray):
    """
    Parses GATT Heart Rate Measurement data (Bluetooth SIG Standard).
    """
    flags = data[0]

    # 1. Determine Heart Rate Format (bit 0 of flags)
    # 0 = 8-bit uint, 1 = 16-bit uint
    hr_format = flags & 0x01
    if hr_format == 0:
        heart_rate = data[1]
        offset = 2
    else:
        heart_rate = int.from_bytes(data[1:3], byteorder='little')
        offset = 3

    # 2. Check if Energy Expended field is present (bit 3)
    if flags & 0x08:
        offset += 2  # Skip 2 bytes of Energy Expended

    # 3. Check if RR-Intervals are present (bit 4)
    rr_intervals = []
    if flags & 0x10:
        while offset < len(data):
            # RR-interval values are in units of 1/1024 seconds
            rr_raw = int.from_bytes(data[offset:offset+2], byteorder='little')
            rr_ms = round((rr_raw / 1024.0) * 1000)
            rr_intervals.append(rr_ms)
            offset += 2

    # Print parsed output
    output = f"Heart Rate: {heart_rate} BPM"
    if rr_intervals:
        output += f" | RR Intervals: {rr_intervals} ms"
    print(output)

async def _main():
    print(f"Scanning for '{DEVICE_NAME_PREFIX}' ...")
    devices = await BleakScanner.discover(timeout=10.0)
    matches = [d for d in devices if d.name and d.name.startswith(DEVICE_NAME_PREFIX)]
    if not matches:
        print(f"No device matching '{DEVICE_NAME_PREFIX}' found. Is it powered on and nearby?")
        return
    device = matches[0]

    print(f"Found {device.name} ({device.address}). Connecting...")
    async with BleakClient(device) as client:
        print(f"Connected: {client.is_connected}")

        # Subscribe to heart rate notifications
        await client.start_notify(HR_MEASUREMENT_UUID, parse_heart_rate)
        print("Streaming data... Press Ctrl+C to stop.")

        # Keep connection open
        while True:
            await asyncio.sleep(1)

def main():
    try:
        asyncio.run(_main())
    except KeyboardInterrupt:
        print("\nDisconnected.")

