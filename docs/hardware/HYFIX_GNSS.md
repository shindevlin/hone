# Hyfix GNSS Base Station — BTCPC Hardware Reference

## Device Info
- **Model**: Hyfix MobileCM MCMv3
- **Firmware**: MCMv3-2.3.3@20251019
- **Hardware**: MCMv3 (hw v2.0.4.0)
- **GNSS**: Triple-Band (UM980R4.10Build13375)
- **Serial**: E831CD2ABD71
- **IP**: 192.168.68.75 (DHCP, WiFi — intermittent)
- **WiFi**: 2.4 GHz (connected to "Hale la Puerta")
- **WiFi Signal**: -62 dBm (needs external antenna)
- **BTCPC Account**: natoshisakamoto (sensor: natoshisakamoto/gnss-base)
- **Status**: TRANSMITTING (when WiFi is up)

## What It Does
- Receives GNSS satellite signals (GPS, GLONASS, Galileo, BeiDou)
- Computes RTK correction data (RTCM3 format)
- Streams corrections to GEODNET server (52.8.236.207:2201 UDP)
- BTCPC intercepts the stream and forwards to additional services

## Web API (port 80)

### Authentication
```
POST /login
Body: mkey=<miner_key>
Response: cookie value (use as USER cookie)
```

### Status Endpoint
```
GET /devStatus (requires USER cookie)
Response: {
  "role": "0",
  "wifiStrength": "-62",
  "nsStatus": "TRANSMITTING",
  "upDateRate": "933.29",
  "gnssDataRate": "990.29",
  "ts": "2026-04-13 15:45:57",
  "uptime": "2186(secs)",
  "fwVer": "MCMv3-2.3.3@20251019",
  "hwVer": "MCMv3",
  "sn": "E831CD2ABD71",
  "gnss": "Triple-Band",
  "gnssVer": "UM980R4.10Build13375"
}
```

### Key Fields
| Field | Meaning |
|-------|---------|
| nsStatus | TRANSMITTING = sending corrections, SEARCHING = acquiring sats |
| upDateRate | corrections per second (higher = better) |
| gnssDataRate | raw GNSS data rate |
| wifiStrength | dBm (below -70 = weak, needs antenna) |
| uptime | seconds since boot |

## BTCPC Integration

### btcpc-gnss-bridge (runs on natoshi PC)
- Polls /devStatus every 30 seconds
- Submits SENSOR_READING entries to BTCPC chain
- Sensor ID: natoshisakamoto/gnss-base
- Type: gps, unit: gnss_correction
- Earns from 10% IoT pool

### btcpc-gnss-relay (runs on natoshi PC, requires sudo)
- ARP spoofs to intercept RTCM3 UDP stream from Hyfix to GEODNET
- Forwards copies to multiple NTRIP casters:
  - RTK Direct (ntrip.rtkdirect.com:2101)
  - onocoy (servers.onocoy.com:2121, needs TLS)
- Records RTCM frame metadata on BTCPC chain
- **NOTE**: ARP spoofing only works on same layer-2 segment. WiFi↔ethernet bridge may not work.

### Environment Variables (.env on natoshi)
```
BTCPC_GNSS_HOST=192.168.68.75
BTCPC_GNSS_DEVICE_IP=192.168.68.75
BTCPC_GNSS_GATEWAY_IP=192.168.68.1
BTCPC_GNSS_WIFI_IFACE=enp0s31f6
BTCPC_GNSS_MINER_KEY=<in env>
BTCPC_ONOCOY_USER=<in env>
BTCPC_ONOCOY_PASS=<in env>
BTCPC_RTK_USER=<in env>
BTCPC_RTK_PASS=<in env>
BTCPC_RTK_MOUNT=<in env>
```

## Earning From Multiple Networks

| Service | Token | Protocol | Status |
|---------|-------|----------|--------|
| BTCPC | BTCPC | HTTP poll → chain | bridge running (when WiFi up) |
| RTK Direct | RTK | NTRIP TCP | connected (409 conflict clears after session timeout) |
| GEODNET | GEOD | UDP direct | waiting on support to deregister previous owner |
| onocoy | ONO | NTRIP TLS | needs TLS support in relay (ECONNRESET) |

## Known Issues
- WiFi drops frequently — needs external 2.4GHz antenna (SMA connector available)
- Web server sometimes unresponsive even when pingable — power cycle fixes it
- ARP spoofing doesn't work across WiFi↔ethernet bridge on this router
- GEODNET account locked to previous owner — support ticket pending
- onocoy needs TLS (tls: true in caster config) — recently fixed in code, untested

## Hardware Connections
- **Antenna**: GNSS multi-band antenna (roof-mounted for sky view)
- **WiFi antenna**: internal (SMA port available for external upgrade)
- **Power**: USB-C, continuous power required
- **No Ethernet**: WiFi only
