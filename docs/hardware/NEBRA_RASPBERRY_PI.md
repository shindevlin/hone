# Nebra Raspberry Pi — BTCPC Hardware Reference

## Device Info
- **Model**: Nebra Indoor Helium Hotspot (repurposed)
- **Board**: Raspberry Pi CM3+ (or Pi 4 equivalent)
- **CPU**: ARM Cortex-A53 @ 1.4GHz (quad-core)
- **RAM**: 1GB
- **OS**: Raspberry Pi OS Bookworm (Debian 12)
- **IP**: 192.168.68.75 (static, wired ethernet)
- **SSH**: pi@192.168.68.75 (password in env)
- **WiFi**: disabled (unreliable, wired instead)
- **Node**: /usr/local/bin/node v20.20.2
- **BTCPC Account**: shindevlin
- **Roles**: clock, storage, gateway

## Hardware Interfaces

### LoRa Concentrator (SX1302)
- **SPI**: /dev/spidev0.0 or /dev/spidev0.1
- **Frequencies**: US915 (915 MHz), EU868, AU915, AS923
- **Status**: detected but no sensors deployed yet
- **BTCPC daemon**: btcpc-nebra (gateway role)
- **Packet format**: Cayenne LPP via Semtech UDP on port 1700

### GPIO
- **Available**: standard Raspberry Pi 40-pin header
- **I2C**: GPIO 2 (SDA), GPIO 3 (SCL) — for BME280, ADXL345, etc.
- **SPI**: GPIO 10/9/11/8 — used by LoRa concentrator
- **UART**: GPIO 14 (TX), GPIO 15 (RX) — for GPS, PMS5003
- **ADC**: none built-in (needs external ADC like ADS1115)

### USB Ports
- 4x USB 2.0 available for:
  - RTL-SDR dongle (ADS-B air traffic)
  - USB GNSS receiver
  - Additional sensors

### Storage
- microSD (boot) + optional USB SSD
- BTCPC-FS blob storage on port 4243

### Network
- Ethernet: 100Mbps (wired, static IP)
- WiFi: disabled (was unreliable)

## BTCPC Services (systemd)

```
/etc/systemd/system/btcpc.service
- Type: simple
- User: pi
- WorkingDirectory: /home/pi/btcpc
- ExecStart: /usr/local/bin/node bin/btcpc-all
- Restart: always (10s delay)
- Auto-start on boot: enabled
```

## Roles Running

### Clock (5% reward pool)
- Sends CLOCK_HEARTBEAT every epoch (30s)
- Connected to relay: wss://btcpc-relay.shindevlin.workers.dev/ws
- P2P port: 6943

### Storage (12% reward pool)
- BTCPC-FS blob host on port 4243
- Capacity: 10GB configured
- Heartbeats sent each epoch

### Gateway (10% IoT pool)
- btcpc-nebra daemon running
- Listens on UDP port 1700 for LoRa packets
- Reports onboard sensors to chain every 30s:
  - CPU temperature (/sys/class/thermal/thermal_zone0/temp)
  - System load average (os.loadavg)
  - Disk usage (df)
  - LoRa radio status (SPI device present check)
- Submits via POST /api/sensors/:id/readings to natoshi API (192.168.68.72:3000)
- Sensor IDs: shindevlin/nebra-1-cpu-temp, shindevlin/nebra-1-load, etc.

## Auto-Update
- Core account (shindevlin) — checks every 5 minutes
- btcpc-update-source daemon pulls from GitHub
- Health check + rollback on failure

## Self-Heal Watchdog
- Checks every 5 minutes if latest block is >10 minutes old
- Restarts all children if chain is stalled
- systemd restarts btcpc.service if process crashes

## Sensor Expansion (what can be plugged in)

| Sensor | Interface | GPIO Pins | Driver |
|--------|-----------|-----------|--------|
| BME280 (temp/humidity/pressure) | I2C | GPIO 2, 3 | i2c-tools + python |
| PMS5003 (air quality) | UART | GPIO 14, 15 | serial @ 9600 |
| ADXL345 (vibration) | I2C | GPIO 2, 3 | i2c-tools |
| RTL-SDR (ADS-B) | USB | USB port | dump1090 |
| GPS module | UART | GPIO 14, 15 | gpsd |
| INA219 (power) | I2C | GPIO 2, 3 | i2c-tools |

## Known Issues
- Port 4243 conflict on restart (old storage process holds port, resolves after timeout)
- LoRa concentrator detected but untested with actual sensors
- npm install is slow on ARM (~5 minutes)
