# BTCPC Hardware Product Line
**Bitcoin Proof of Compute — Physical Network Nodes**
*Shin Devlin — April 2026*

---

## Overview

BTCPC hardware devices are physical nodes that earn BTCPC passively by contributing real-world data to the network: air quality, weather, seismic activity, noise pollution, GPS mobility, and more. Every device is a mining node, a sensor array, and a data relay simultaneously.

The business model is two-sided:
- **Device owners** earn BTCPC rewards for data contribution
- **The protocol** sells aggregated B2B data to weather services, city governments, health insurers, and research institutions — and returns 70% of that revenue to device owners as additional BTCPC rewards

No subscription. No cloud dependency. The device connects to the BTCPC network directly and earns from day one.

---

## The Three Tiers

### Micro — "Forget It Exists"

**Form factor**: Credit card thickness. Keychain, pocket, bag clip.  
**Retail price**: ~$80  
**Cell plan required**: No — reports via BLE to nearby phone or General device, LoRa to any nearby gateway when out of BLE range

The Micro signs every sensor reading with its own registered device key. Any relay (phone, General node, public gateway) can forward the signed packet to the chain. The Micro earns regardless of which relay forwarded it. No trust required from the relay.

#### Sensors

| Sensor | Chip | Data produced |
|--------|------|---------------|
| Temperature, humidity, pressure, VOC/gas | BME688 | Hyperlocal weather + air quality proxy |
| 6-axis motion (accelerometer + gyro) | LSM6DSO | Activity, vibration, distributed seismic |
| Ambient light | VEML7700 | Light pollution mapping |

#### Radios

| Radio | Purpose |
|-------|---------|
| BLE 5.3 | Pairs with phone app or General device — primary relay |
| LoRa (SX1262) | Fallback — reaches any TTN / Helium / BTCPC gateway within 2–5 km |

#### Power
- 500 mAh LiPo, fully sealed, Qi wireless charging
- 3–4 days per charge under normal use

#### Bill of Materials

| Component | Cost |
|-----------|------|
| nRF52840 SoC (BLE 5.3) | $5.00 |
| SX1262 LoRa module | $4.00 |
| BME688 environmental | $4.00 |
| LSM6DSO IMU | $2.00 |
| VEML7700 light sensor | $1.50 |
| PMIC + 500 mAh LiPo | $6.00 |
| Qi wireless charging | $3.00 |
| PCB + antenna + passives | $5.00 |
| Enclosure (IP54) | $5.00 |
| **Total BOM** | **~$35.50** |

Retail at ~2.25× BOM: **$80**

---

### General — "Always in Your Pocket"

**Form factor**: Similar to Flipper Zero — 100×42×24 mm, plastic enclosure, monochrome screen, 5-way navigation, hackable and open source. The same demographic: technically curious people who want a device that does something real.  
**Retail price**: ~$220  
**Cell plan required**: No — uses WiFi when available, BLE to phone, LoRa as fallback. No SIM required.

The General is the network's workhorse. It earns from 6 sensor categories simultaneously, acts as a LoRa gateway for nearby Micros, and can run custom sensor plugins via its open SDK.

#### Sensors

| Sensor | Chip | Data produced |
|--------|------|---------------|
| Temperature, humidity, pressure, VOC/gas | BME688 | Full environmental + air quality |
| Real CO2 (NDIR, not estimated) | SCD40 | Actual CO2 concentration |
| 6-axis IMU | LSM6DSO | Motion, steps, vibration, seismic |
| Magnetometer | LIS3MDL | Geomagnetic field, indoor positioning |
| Ambient light + UV index | VEML7700 + VEML6075 | Light and UV pollution |
| MEMS microphone (SPL only — no audio stored) | SPH0645 | Noise pollution dB level |
| NFC | PN532 | IoT access, device pairing |

#### Radios

| Radio | Purpose |
|-------|---------|
| BLE 5.3 | Phone app, Micro relay, device pairing |
| LoRa SX1262 | Gateway for nearby Micros, long-range fallback |
| WiFi (ESP32 built-in) | Primary uplink when available, OTA firmware updates |
| Sub-GHz 433/915 MHz | Compatible with existing IoT sensor ecosystems |

#### Display + Interface
- 128×64 monochrome OLED (or low-power e-ink variant)
- 5-way navigation button + back
- USB-C for charging and debug
- Open hardware + open SDK — community plugin ecosystem

#### Power
- 2000 mAh LiPo, USB-C charging, optional Qi
- 4–6 days per charge

#### Bill of Materials

| Component | Cost |
|-----------|------|
| ESP32-S3 (WiFi + BLE + sub-GHz) | $4.00 |
| nRF52840 co-processor (BLE + LoRa management) | $5.00 |
| SX1262 LoRa | $4.00 |
| BME688 | $4.00 |
| SCD40 CO2 | $12.00 |
| LSM6DSO + LIS3MDL | $3.50 |
| VEML7700 + VEML6075 | $3.00 |
| SPH0645 mic | $1.50 |
| PN532 NFC | $3.00 |
| OLED display | $3.00 |
| PMIC + 2000 mAh LiPo | $9.00 |
| Qi wireless charging | $3.50 |
| PCB (4-layer) + antennas | $9.00 |
| Enclosure + buttons | $10.00 |
| Misc passives + connectors | $4.00 |
| **Total BOM** | **~$82.50** |

Retail at ~2.65× BOM: **$220**

---

### Macro — "Set It and Forget It"

**Form factor**: IP65 weatherproof enclosure with a Raspberry Pi CM4 inside. Mounts on a rooftop, windowsill, balcony railing, or carried in a backpack with a shoulder-strap solar panel. Hot-swappable 18650 battery sled — standard cells available at any hardware store or vape shop.  
**Retail price**: ~$500  
**Power**: 12V DC input, XT60 solar/DC port, 10W solar panel runs it indefinitely when mounted outdoors

This is a full BTCPC network node. It runs the complete BTCPC node software on Linux, earns mining rewards, sensor rewards, storage rewards, and clock rewards simultaneously. It acts as a LoRa gateway for nearby Micro and General devices.

#### Compute
- **Raspberry Pi CM4** (2 GB RAM, 16 GB eMMC) on a custom carrier board
- Full Linux — Node.js BTCPC software runs natively
- SSH access, OTA via standard Linux tooling
- Alternatively: Pi Zero 2W for cost-sensitive deployments (~$15 vs ~$35 for CM4)

#### Sensors

| Sensor | Chip | Data produced |
|--------|------|---------------|
| Particulate matter PM1 / PM2.5 / PM10 | SPS30 or SEN55 | Real air quality — what PurpleAir sells |
| Real CO2 (high accuracy) | SCD41 | Calibrated CO2 concentration |
| Temperature, humidity, pressure, VOC | BME688 | Full environmental |
| Wind speed + direction | RS-485 anemometer | Weather station grade |
| Rain gauge | Tipping bucket | Precipitation measurement |
| 8-channel spectral light sensor | AS7341 | Full spectrum, not just UV index |
| 9-axis high-precision IMU | ICM-42688 | Seismic sensitivity, high dynamic range |
| MEMS microphone array (4×) | SPH0645 ×4 | Directional noise pollution mapping |
| Optional: camera | Pi Camera Module 3 | Visual air quality, traffic counting, visibility |

#### Radios

| Radio | Purpose |
|-------|---------|
| LTE Cat-4 modem (SIM7600) | Primary uplink — direct to BTCPC network, no WiFi needed |
| WiFi 6 + BT 5.0 | Built into CM4 — local network, BLE to nearby devices |
| LoRa SX1262 | Gateway for Micro + General devices within 5 km |
| Optional: 915 MHz sub-GHz | Extended IoT sensor compatibility |

#### Power System

| Component | Detail |
|-----------|--------|
| Main input | 12V DC, XT60 barrel jack |
| Solar input | 10W panel (included or optional) — MPPT charge controller onboard |
| Battery | Hot-swap 18650 sled, 2S2P (4 × 18650 = ~15 Ah) |
| Runtime | 3–5 days on battery, indefinite with 10W solar |
| Hot-swap buffer | Supercapacitor holds Pi state for 5 seconds during cell swap |

Standard 18650 cells (Samsung 30Q, LG HG2, etc.) available worldwide for $4–6 each. Never stranded.

#### Bill of Materials

| Component | Cost |
|-----------|------|
| Raspberry Pi CM4 2GB/16GB | $35.00 |
| Custom carrier board | $12.00 |
| SIM7600 LTE modem | $18.00 |
| SX1262 LoRa | $4.00 |
| SPS30 particulate sensor | $20.00 |
| SCD41 CO2 | $14.00 |
| BME688 | $4.00 |
| Anemometer (RS-485) | $15.00 |
| Rain gauge (tipping bucket) | $12.00 |
| AS7341 spectral sensor | $5.00 |
| ICM-42688 IMU | $3.50 |
| SPH0645 mic array (×4) | $6.00 |
| 10W MPPT solar charger | $8.00 |
| 18650 battery sled (4 cells included) | $20.00 |
| IP65 enclosure + weatherproofing | $18.00 |
| Antennas (LTE, LoRa, GPS) | $8.00 |
| PCB + misc | $10.00 |
| **Total BOM** | **~$182.50** |

Retail at ~2.75× BOM: **$500**

---

## How Devices Earn

Every device earns on multiple dimensions simultaneously:

| Reward type | Micro | General | Macro |
|-------------|-------|---------|-------|
| Sensor data rewards | Yes | Yes | Yes |
| Mining (inference work) | No | No | Yes (full node) |
| Storage hosting | No | No | Yes (full node) |
| Clock node | No | No | Yes (full node) |
| LoRa gateway relay fees | No | Yes | Yes |
| Cross-chain wBTCPC credits | Yes | Yes | Yes |

Cross-chain credits: every BTCPC earned generates 0.1 BTCPC claimable as wBTCPC on each of 10 supported chains (Ethereum, Base, Arbitrum, Optimism, Solana, TON, Bitcoin, Hive, BSC, Polygon). A device earning 1 BTCPC/day also accrues 1.0 BTCPC/day in cross-chain credits across those chains.

---

## Data Verification

Sensor data is not taken on faith. Three-layer verification:

1. **Device key signing**: every reading is signed with a registered secp256k1 device key. Fabricated readings require a registered device with staked BTCPC behind it — raising the cost of spoofing.

2. **Cross-corroboration**: readings from devices in the same geographic area are compared. A device reporting 15°C in Phoenix in July, or PM2.5 of 0 µg/m³ in downtown Los Angeles, gets flagged. An attacker would need to control a neighborhood's worth of devices to fake plausible data.

3. **Covariance fingerprinting**: real sensor data has correlated noise — temperature drift affects humidity in physically predictable ways. Synthetic data that's too clean fails statistical tests. Motion data that lacks the characteristic quantization and noise floor of real MEMS sensors is detectable by ML classifiers.

---

## Data Buyers and Market

The data collected by BTCPC devices has established commercial buyers today:

| Data category | Primary buyers | Market evidence |
|---------------|----------------|-----------------|
| Air quality (PM2.5, CO2, VOC) | City governments, EPA, IQAir, Breezometer, health insurers, real estate | PurpleAir acquired for ~$25M with 70k sensors |
| Hyperlocal weather | Tomorrow.io, AccuWeather, agriculture, energy | Tomorrow.io raised $200M for hyperlocal weather |
| GPS mobility / traffic | HERE, TomTom, urban planners, logistics | HERE valued at ~$1B; pays per-device for probe data |
| Noise pollution | City governments, WHO, real estate | Cities pay $20–50k per noise study |
| Seismic (distributed) | USGS, national earthquake networks | ShakeAlert partnership potential |
| Light / UV | Solar installers, climate researchers, dermatology | Niche but consistent demand |

The protocol sells data via a B2B API. Device owners receive 70% of revenue as BTCPC rewards. The protocol retains 30%.

---

## Business Forecast

### Assumptions
- Hardware gross margin: 40%
- Data revenue split: 70% to device owners (BTCPC rewards), 30% to protocol
- Average data value per active device per year: $80 (Y1) → $180 (Y2) → $250 (Y3)
- Unit mix: 60% Micro, 30% General, 10% Macro by volume

### Unit Sales Forecast

| Year | Micro | General | Macro | Total units | Total hardware revenue |
|------|-------|---------|-------|-------------|----------------------|
| Y1 | 5,000 | 2,000 | 300 | 7,300 | $984k |
| Y2 | 30,000 | 12,000 | 2,000 | 44,000 | $5.84M |
| Y3 | 120,000 | 50,000 | 8,000 | 178,000 | $16.4M |

### Revenue Forecast

| Year | Hardware margin | Data marketplace cut (30%) | **Total gross revenue** |
|------|----------------|---------------------------|------------------------|
| Y1 | $394k | $175k | **$569k** |
| Y2 | $2.34M | $2.38M | **$4.72M** |
| Y3 | $6.56M | $13.35M | **$19.91M** |

**By Year 3, data revenue exceeds hardware revenue.** This is the right shape — hardware is customer acquisition, data is the annuity.

### Comparable companies at 50–170k devices

| Company | Device count | Outcome |
|---------|-------------|---------|
| PurpleAir | ~70k air quality sensors | Acquired ~$25M |
| WeatherFlow / Tempest | ~50k weather stations | Valued ~$30M |
| Helium | ~900k LoRa gateways | Token market cap peaked ~$5B |
| AirGradient | ~10k DIY air quality | Bootstrap profitable |

A BTCPC network of 170k devices with 10 sensor categories, a sovereign chain, cross-chain wBTCPC on 10 networks, and a verified data marketplace is in a different category than any of the above.

---

## Go-to-Market

**Phase 1 — Developer / early adopter**
- Open hardware files on GitHub
- Micro and General available as DIY kits first (lower unit cost, community builds trust)
- Flipper Zero community, Meshtastic community, home weather station hobbyists
- BTCPC token rewards as the primary acquisition incentive

**Phase 2 — Consumer retail**
- Finished retail units on btcpc.network/shop
- Amazon listing for Micro and General
- Bundle: "BTCPC Starter Pack" — 1 General + 3 Micros for $450

**Phase 3 — B2B / city deployments**
- Macro units sold to city governments, universities, research institutions
- "Deploy 100 sensors across your city" program — data stays on BTCPC chain, city gets API access
- Revenue share: city gets 20% of their nodes' data revenue back as service credit

**Phase 4 — OEM / white label**
- Sensor hardware licensed to other IoT manufacturers
- wBTCPC integration as a revenue-sharing protocol layer on top of existing sensor networks

---

## Open Questions

- **Certification**: FCC Part 15 (USA), CE (EU), IC (Canada) required for commercial sale. Budget $15–30k per device per region for certification testing.
- **Contract manufacturing**: Shenzhen EMS at 10k+ units; JLCPCB / PCBWay for initial prototypes
- **SIM for Macro**: Global eSIM (Hologram, Eseye) at $2–5/month per device, or negotiate with carriers at volume
- **Firmware**: nRF52840 firmware in Zephyr RTOS (open source, Nordic-supported); Pi firmware is standard Linux + BTCPC node software
- **Regulatory for sensor data sales**: GDPR/CCPA compliance required for any location data — aggregate and anonymize before selling GPS mobility data
