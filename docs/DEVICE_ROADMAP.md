# BTCPC Device Roadmap

This roadmap prioritizes devices that give BTCPC the most useful, trustworthy, and easy-to-integrate real-world data first. It also reflects the current build reality: a Raspberry Pi is already in the network, so the immediate path is to add sensors and receivers that can plug into a Pi over USB, GPIO, I2C, or serial.

## Priority Table

| Priority | Device | Why it matters | Interface | Price | Purchase |
|---|---|---|---|---|---|
| 1 | RTL-SDR USB dongle | ADS-B air-traffic capture — public, time-sensitive, high-value data | USB | $20-35 | [RTL-SDR.com](https://www.rtl-sdr.com/buy-rtl-sdr-dvb-t-dongles/) |
| 1 | 1090 MHz ADS-B antenna | Improves aircraft reception for useful air-traffic station | RF | $15-30 | [RTL-SDR bundle](https://www.rtl-sdr.com/radarbox-optimized-ads-b-antenna-rtl-sdr-bundle-sale-39-95-shipping/) |
| 2 | BME280 weather sensor | Temperature, humidity, pressure — core environmental baseline | I2C/SPI | $10-15 | [RobotShop](https://www.robotshop.com/products/waveshare-bme280-environmental-sensor-temperature-humidity-barometric-pressure) |
| 2 | Pimoroni Enviro | All-in-one weather + environmental package | Pi HAT | $25-35 | [Pimoroni](https://shop.pimoroni.com/products/enviro) |
| 3 | PMS5003 / PMS7003 | Particulate matter for pollution, dust, smoke monitoring | Serial | $20-30 | [Oz Robotics HAT](https://ozrobotics.com/shop/sensor-air-monitoring-hat-for-raspberry-pi-pmsa003/) |
| 3 | MH-Z19B CO2 sensor | Indoor/outdoor air quality context | Serial/UART | $20-30 | Amazon/AliExpress |
| 3 | SGP30 / CCS811 | VOC/eCO2 indoor air quality | I2C | $10-20 | Adafruit/Amazon |
| 4 | ADXL345 accelerometer | Vibration/motion for structural or seismic activity | I2C/SPI | $5-10 | Adafruit/Amazon |
| 4 | Grove D7S vibration sensor | Seismic/vibration-oriented option | I2C/Grove | $6-10 | [The Pi Hut](https://thepihut.com/products/grove-d7s-vibration-sensor) |
| 4 | Raspberry Shake RS1D | Professional seismic station for earthquake/infrasound | Networked | $385+ | [Raspberry Shake](https://shop.raspberryshake.org) |
| 5 | HC-SR04 ultrasonic | Water level, tank level, flood monitoring | GPIO | $3-8 | Amazon/AliExpress |
| 5 | Capacitive soil moisture | Agriculture, land monitoring, drought conditions | Analog/GPIO | $2-5 | Amazon/AliExpress |
| 5 | Float switch / flood sensor | Water event detection for flood or tank status | GPIO | $5-10 | Amazon |
| 6 | INA219 current sensor | DC power, solar, battery system monitoring | I2C | $10 | [Adafruit](https://www.adafruit.com/product/904) |
| 6 | SCT-013 current clamp | Non-invasive AC current sensing for energy logging | Analog/ADC | $5-15 | Amazon/AliExpress |
| 6 | PZEM-004T energy meter | Voltage/current/power/energy logging for utilities | Serial | $15-25 | Amazon/AliExpress |
| 7 | ESP32 sensor node | Remote sensing stations at low cost | Wi-Fi | $5-15 | Amazon/AliExpress |
| 7 | LoRa module/HAT | Long-range, low-power sensor links | SPI/UART | $15-30 | Amazon/AliExpress |
| 7 | GPS module | Station geolocation and timestamp confidence | UART | $8-15 | Amazon |

## Why each phase matters

- **Phase 1**: Fastest win — ADS-B air traffic is mature, passive, high-value, fits an information blockchain perfectly
- **Phase 2**: Environmental baseline — minimum context data before adding specialized sensors
- **Phase 3**: Public health relevance — pollution and air quality data has real commercial buyers
- **Phase 4-7**: Broader physical-world telemetry network

## What the current build supports

The Raspberry Pi gateway already supports:
- USB devices (RTL-SDR dongles)
- Serial sensors (PMS5003, MH-Z19B)
- I2C sensors (BME280, INA219)
- GPIO sensors (ultrasonic, soil moisture)
- Future expansion via ESP32 or LoRa nodes

No redesign needed — plug in and start earning.

## Suggested first purchase bundle

Best first batch (~$60-80 total):
1. **RTL-SDR USB dongle + 1090 MHz antenna** — air traffic
2. **BME280 weather sensor** — temperature, humidity, pressure
3. **PMS5003 air quality sensor** — particulate matter
4. **ADXL345 or Grove D7S** — vibration/motion

Gives BTCPC a strong mix of **air, weather, pollution, and motion** data with minimal integration risk.
