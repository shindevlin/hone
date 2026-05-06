# BTCPC Hardware Manufacturing Analysis
**Component Sourcing, JIT Feasibility, and Unit Economics**
*Shin Devlin — April 2026*

---

## Executive Summary

All three BTCPC hardware tiers (Micro, General, Macro) can be built entirely from off-the-shelf, commercially available components. No custom silicon is required. PCB fabrication and SMT assembly can be outsourced to contract manufacturers with 2–5 week lead times at prototype volumes, scaling to 1–2 week turnaround at production volumes.

True JIT (just-in-time) is partially achievable — PCB assembly is fast, but two sensor components (Sensirion SPS30 particulate, SCD40/41 CO2) carry 8–12 week lead times at volume and must be managed with safety stock. Everything else is commodity.

**The critical manufacturing decision:** use pre-certified radio modules (not bare chips) for the Micro and General. This costs $3–6 more per unit but eliminates $15–30k in FCC/CE certification cost per product. The break-even is under 5,000 units — a straightforward call.

---

## Component Availability Analysis

### Micro — All Off-the-Shelf

| Component | Chip | Availability | Lead time | Source | Unit cost (1k) | Unit cost (10k) |
|-----------|------|-------------|-----------|--------|----------------|-----------------|
| BLE + LoRa SoC | Nordic nRF52840 (bare) | Good | 4–8 wk | DigiKey, Mouser | $4.20 | $3.10 |
| BLE module (pre-certified) | Raytac MDBT50Q-RX | Good | 2–4 wk | Mouser, direct | $7.50 | $5.80 |
| LoRa module (pre-certified) | EBYTE E22-900M22S | Good | 1–2 wk | LCSC, AliExpress | $3.80 | $2.90 |
| Environmental | Bosch BME688 | Good | 1–3 wk | DigiKey, Mouser, LCSC | $3.20 | $2.40 |
| IMU | ST LSM6DSO | Good | 1–2 wk | DigiKey, Mouser | $1.80 | $1.30 |
| Light sensor | Vishay VEML7700 | Good | 1–2 wk | DigiKey, Mouser | $0.85 | $0.62 |
| PMIC | TI BQ25185 | Good | 2–4 wk | DigiKey, TI direct | $1.20 | $0.88 |
| Qi RX | TI BQ51013B | Good | 2–4 wk | DigiKey, Mouser | $1.40 | $1.05 |
| 500 mAh LiPo | Generic 503035 | Good | 1–2 wk | LCSC, Alibaba | $2.80 | $1.95 |

**Risk:** None. Every component is commodity with multiple second-source suppliers.

**Certification note:** Using the Raytac MDBT50Q-RX (pre-FCC/CE certified module) instead of bare nRF52840 chip adds $3.30/unit at 1k but eliminates the radio certification testing requirement (~$15–20k). The PCB itself still needs FCC Part 15 unintentional emissions testing (~$3–5k), but that is fast and cheap.

---

### General — All Off-the-Shelf

| Component | Chip | Availability | Lead time | Source | Unit cost (1k) | Unit cost (10k) |
|-----------|------|-------------|-----------|--------|----------------|-----------------|
| Main MCU + WiFi | Espressif ESP32-S3-WROOM-1 (module) | Excellent | 1–2 wk | LCSC, DigiKey, Mouser | $3.20 | $2.35 |
| BLE co-processor | Nordic nRF52840 (Raytac module) | Good | 2–4 wk | Mouser | $7.50 | $5.80 |
| LoRa | EBYTE E22-900M22S | Good | 1–2 wk | LCSC | $3.80 | $2.90 |
| Environmental + VOC | Bosch BME688 | Good | 1–3 wk | DigiKey | $3.20 | $2.40 |
| Real CO2 (NDIR) | Sensirion SCD40 | **Moderate** | **8–12 wk** | Mouser, Digi-Key | $9.50 | $7.20 |
| IMU | ST LSM6DSO | Good | 1–2 wk | DigiKey | $1.80 | $1.30 |
| Magnetometer | ST LIS3MDL | Good | 1–2 wk | DigiKey | $1.10 | $0.82 |
| Ambient light | Vishay VEML7700 | Good | 1–2 wk | DigiKey | $0.85 | $0.62 |
| UV index | Vishay VEML6075 | Good | 1–2 wk | DigiKey | $1.10 | $0.82 |
| MEMS mic | Knowles SPH0645 | Good | 2–4 wk | DigiKey, Mouser | $1.20 | $0.88 |
| NFC | NXP PN532 | Good | 2–4 wk | DigiKey, LCSC | $2.40 | $1.75 |
| OLED display | 128×64 SSD1306 | Excellent | 1 wk | LCSC, AliExpress | $1.80 | $1.20 |
| PMIC | TI BQ25895 | Good | 2–4 wk | DigiKey | $1.65 | $1.22 |
| Qi RX | TI BQ51013B | Good | 2–4 wk | DigiKey | $1.40 | $1.05 |
| 2000 mAh LiPo | Generic 604060 | Good | 1–2 wk | LCSC, Alibaba | $4.20 | $3.10 |

**Risk: SCD40 CO2 sensor.** Sensirion's CO2 sensors are single-sourced (no second source for NDIR CO2 at this price point) and carry long lead times at volume. Must maintain 12–16 weeks of safety stock. At 10k units/year, this means keeping ~2,500 units of SCD40 in inventory at all times.

**Alternative:** At low volumes, the SCD40 can be swapped for the Sensirion SGP41 (VOC + NOx proxy, not true NDIR CO2) for $3.50 vs $9.50. Degrades CO2 data quality but removes the supply risk. Offer SCD40 as an upgrade SKU.

**Certification note:** ESP32-S3-WROOM-1 is pre-FCC/CE certified. Combined with the Raytac nRF52840 module and EBYTE LoRa module, all three radios come pre-certified. The General PCB needs only FCC Part 15 emissions testing (~$3–5k) rather than full radio certification.

---

### Macro — Off-the-Shelf with One Supply Risk

| Component | Chip/Module | Availability | Lead time | Source | Unit cost (100) | Unit cost (1k) |
|-----------|-------------|-------------|-----------|--------|-----------------|----------------|
| Compute | Raspberry Pi CM4 2GB/16GB | **Good (was constrained)** | 2–4 wk | RS Components, approved resellers | $35.00 | $32.00 |
| Carrier board (custom PCB) | Custom | N/A | 2 wk | JLCPCB, PCBWay | $18.00 | $10.00 |
| LTE modem | SIMCom SIM7600G-H module | Good | 2–4 wk | LCSC, Mouser | $16.50 | $12.80 |
| LoRa | EBYTE E22-900M33S (higher power) | Good | 1–2 wk | LCSC | $5.20 | $3.90 |
| Particulate PM2.5 | Sensirion SPS30 | **Moderate** | **8–12 wk** | Mouser, DigiKey | $18.50 | $14.20 |
| High-accuracy CO2 | Sensirion SCD41 | **Moderate** | **8–12 wk** | Mouser, DigiKey | $12.80 | $9.80 |
| Environmental | Bosch BME688 | Good | 1–3 wk | DigiKey | $3.20 | $2.40 |
| Wind speed/direction | RS485 anemometer (generic) | Good | 2–3 wk | Alibaba, AliExpress | $12.00 | $8.50 |
| Rain gauge | Tipping bucket (generic) | Good | 2–3 wk | Alibaba | $9.50 | $7.00 |
| Spectral light | AMS AS7341 | Good | 2–4 wk | DigiKey, Mouser | $4.20 | $3.10 |
| Precision IMU | TDK ICM-42688 | Good | 2–4 wk | DigiKey | $3.10 | $2.30 |
| MEMS mic ×4 | Knowles SPH0645 ×4 | Good | 2–4 wk | DigiKey | $4.80 | $3.52 |
| MPPT solar charger | Voltaic V88 or CN3722 IC | Good | 1–2 wk | DigiKey, LCSC | $7.50 | $5.60 |
| 18650 sled + 4 cells | Samsung 30Q ×4 | Excellent | 1 wk | Battery Junction, 18650batterystore | $16.00 | $12.00 |
| IP65 enclosure | Hammond 1554 series or equiv | Good | 1–2 wk | Mouser, Newark | $14.00 | $10.50 |
| Misc antennas, connectors | Various | Excellent | 1 wk | LCSC | $8.00 | $5.50 |

**Risk: Raspberry Pi CM4 and Sensirion sensors.** CM4 availability has improved significantly since 2023 but still requires ordering through approved distributors. Maintain 8-week safety stock. SPS30 and SCD41 are the same single-source risk as SCD40 on the General — manage with 12-week safety stock.

**Alternative compute:** If CM4 supply is constrained, the Radxa CM3 (CM4-compatible form factor, RK3566 SoC) or Orange Pi CM4 are pin-compatible drop-in replacements at $22–28. The BTCPC node software runs on any ARM Linux board.

---

## Full BOM Cost Analysis at Volume

### Micro

| Volume | Component BOM | PCB + Assembly | Enclosure | Test + QC | **Unit cost** | Retail ($80) | Margin |
|--------|--------------|----------------|-----------|-----------|--------------|--------------|--------|
| 500 | $38.00 | $22.00 | $6.00 | $4.00 | **$70.00** | $80 | 12.5% |
| 1,000 | $34.50 | $16.00 | $5.50 | $3.00 | **$59.00** | $80 | 26.3% |
| 5,000 | $29.00 | $11.00 | $4.50 | $2.00 | **$46.50** | $80 | 41.9% |
| 10,000 | $26.00 | $9.00 | $4.00 | $1.50 | **$40.50** | $80 | 49.4% |
| 50,000 | $21.00 | $7.00 | $3.20 | $1.00 | **$32.20** | $80 | 59.8% |

*PCB + Assembly includes bare board fabrication + SMT pick and place + reflow + inspection.*

### General

| Volume | Component BOM | PCB + Assembly | Enclosure + screen | Test + QC | **Unit cost** | Retail ($220) | Margin |
|--------|--------------|----------------|-------------------|-----------|--------------|---------------|--------|
| 500 | $98.00 | $28.00 | $14.00 | $6.00 | **$146.00** | $220 | 33.6% |
| 1,000 | $88.00 | $20.00 | $12.00 | $5.00 | **$125.00** | $220 | 43.2% |
| 5,000 | $74.00 | $14.00 | $10.00 | $3.50 | **$101.50** | $220 | 53.9% |
| 10,000 | $66.00 | $11.00 | $8.50 | $2.50 | **$88.00** | $220 | 60.0% |
| 50,000 | $54.00 | $8.50 | $7.00 | $1.50 | **$71.00** | $220 | 67.7% |

### Macro

| Volume | Component BOM | PCB + Assembly | Enclosure + power | Test + QC | **Unit cost** | Retail ($500) | Margin |
|--------|--------------|----------------|-------------------|-----------|--------------|---------------|--------|
| 100 | $210.00 | $48.00 | $32.00 | $15.00 | **$305.00** | $500 | 39.0% |
| 500 | $188.00 | $32.00 | $26.00 | $10.00 | **$256.00** | $500 | 48.8% |
| 1,000 | $172.00 | $24.00 | $22.00 | $8.00 | **$226.00** | $500 | 54.8% |
| 5,000 | $148.00 | $16.00 | $18.00 | $5.00 | **$187.00** | $500 | 62.6% |

---

## JIT Manufacturing Feasibility

### What "JIT" Means for Electronics

True JIT (Toyota-style, build only when ordered) is not achievable for electronics due to:
1. PCB fabrication lead time: 3–5 days (JLCPCB fast track) to 10 days (standard)
2. SMT assembly: 5–15 days depending on EMS capacity
3. Long-lead components: 8–12 weeks for Sensirion sensors

**Achievable model: "Assemble to Order" with pre-positioned components**

Split production into three stages:

| Stage | What | Lead time | JIT-able? |
|-------|------|-----------|-----------|
| 1. Component procurement | Buy bulk components to 12-week rolling forecast | 1–12 weeks | No — requires forward buying |
| 2. PCB bare board fab | JLCPCB/PCBWay in batches of 100–500 | 3–5 days | Yes |
| 3. SMT assembly | EMS picks and places all components | 5–10 days | Yes, at reasonable volume |
| 4. Final assembly + test | Enclosure, battery, firmware flash, QC | 1–2 days | Yes |
| 5. eSIM provisioning + device key generation | BTCPC chain registration | Hours | Yes |

**Result:** Once components are in stock, a new device can go from bare PCB to shipped in 7–14 days. This is fast enough for most sales channels (direct website, Amazon FBA with buffer stock).

### Safety Stock Requirements

| Component | Lead time | Recommended safety stock | Cost at 1k units/month |
|-----------|-----------|--------------------------|------------------------|
| Sensirion SCD40 (General) | 12 weeks | 3 months demand | $28.5k |
| Sensirion SPS30 (Macro) | 12 weeks | 3 months demand | $42.6k |
| Sensirion SCD41 (Macro) | 12 weeks | 3 months demand | $29.4k |
| Raspberry Pi CM4 (Macro) | 4 weeks | 6 weeks demand | $5.3k |
| Raytac nRF52840 module | 4 weeks | 6 weeks demand | $8.7k |
| All other components | 1–3 weeks | 2 weeks demand | ~$15k |

**Total safety stock investment at 1k units/month across all three tiers: ~$130k**

This is the minimum working capital required to manufacture without supply disruptions.

---

## Contract Manufacturing Options

### Prototyping (1–100 units)

**JLCPCB with PCBA service**
- Turn around: 7–15 days including SMT assembly
- Minimum order: 5 units
- Cost: $15–30/board for assembly labor on a 2-layer board; $20–40 on 4-layer
- Includes component sourcing from their parts library (LCSC)
- Best for: first prototypes, developer kits, early backer units
- URL: jlcpcb.com

**PCBWay**
- Similar capability to JLCPCB
- Slightly more expensive but better customer support for complex builds
- Good for Macro carrier board (complex, multi-layer)

### Small Volume (100–2,000 units)

**Seeed Studio Fusion**
- Shenzhen-based, English-speaking, experienced with IoT devices
- PCBA + full assembly + testing + packaging possible
- Lead time: 3–4 weeks
- Good mid-range option between prototype and mass production

**MacroFab (US-based)**
- US domestic manufacturing for FCC pre-compliance testing convenience
- Higher cost ($30–60/board assembly) but no import duties, faster communication
- Good for small runs where US manufacturing matters for optics/compliance

### Medium Volume (2,000–20,000 units)

**Shenzhen EMS (Electronics Manufacturing Services)**
- Companies like Foxlink, Flextronics SMT division, or smaller Shenzhen shops
- At 5k+ units, cost per board drops to $8–15 for assembly
- Requires on-the-ground sourcing agent or direct China presence
- Lead time: 4–6 weeks for first production run, 2–3 weeks for repeat orders
- This is where the business should be at Year 2

**Seeed Studio OEM division**
- Same company as Fusion but dedicated OEM line for >2k units
- Better pricing, dedicated account manager

### Large Volume (20,000+ units)

At this volume, a dedicated ODM (Original Design Manufacturer) partnership makes sense:

- ODM handles component sourcing, PCB, assembly, test, and packaging
- Business provides design files, firmware, and quality spec
- Cost structure shifts from "cost plus" to negotiated per-unit price
- Shenzhen ODM shops experienced with IoT devices: AcSiP, USI, Avnet Embedded
- Lead time for first run: 8–12 weeks. Repeat orders: 3–4 weeks.
- This is the Year 3 manufacturing model.

---

## Certification Costs and Timeline

Required before commercial sale in any major market. Must be budgeted and started early — 3–6 month timeline typical.

| Certification | Market | Required for | Estimated cost | Timeline |
|--------------|--------|-------------|----------------|----------|
| FCC Part 15 (unintentional) | USA | All three tiers | $3–5k per device | 6–10 weeks |
| FCC Part 15 (intentional, radio) | USA | Micro, General | $15–25k per device | 8–16 weeks |
| CE (RED + EMC + LVD) | EU | All three tiers | €8–15k per device | 8–16 weeks |
| IC (ISED) | Canada | All three tiers | ~$5k per device | 4–8 weeks |
| RoHS/REACH | EU | All (materials) | $1–3k per device | 2–4 weeks |
| UN 38.3 (LiPo battery) | Shipping | All (contain LiPo) | $2–5k | 4–6 weeks |

**Cost reduction strategy — use pre-certified radio modules:**

By using pre-FCC/CE certified modules (Raytac nRF52840, EBYTE SX1262, Espressif WROOM), the radio certification cost is reduced to a modular grant. The board only needs FCC Part 15 unintentional emissions testing (~$3–5k) instead of full radio certification (~$15–25k).

**Per-device certification budget:**

| Device | Without pre-cert modules | With pre-cert modules | Savings |
|--------|--------------------------|----------------------|---------|
| Micro | $35–45k | $12–18k | ~$22k |
| General | $45–60k | $15–22k | ~$30k |
| Macro | $25–35k (no new radios) | $20–30k | ~$5k |

**Total certification budget (all three tiers, US + EU): ~$55–85k using pre-certified modules**

This should be budgeted in Year 1 as a fixed cost. Certification is per-design, not per-unit — once certified, the design can be manufactured in unlimited quantity.

---

## Build vs. Buy Analysis

For each tier, there is an "off-the-shelf hardware" alternative that accelerates time to market at the cost of margin:

### Micro — Build (custom PCB is strongly preferred)

| Option | Time to market | Unit cost | Margin at $80 |
|--------|---------------|-----------|---------------|
| Custom PCB (recommended) | 6–9 months | $40.50 (10k) | 49% |
| Nordic Thingy:91 (pre-made dev kit) | 1 month | $99 retail | Negative |
| Seeed XIAO nRF52840 + sensors | 2–3 months (integration) | $55 (built up) | 31% |

Custom PCB wins on margin. Use XIAO for early developer kits and demos while PCB is in certification.

### General — Hybrid approach

| Option | Time to market | Unit cost | Margin at $220 |
|--------|---------------|-----------|----------------|
| Custom PCB (recommended) | 6–9 months | $88.00 (10k) | 60% |
| Flipper Zero + sensor HAT | 2 months (SW only) | $175 (Flipper $169 + sensors) | 20% |
| Custom PCB + Flipper Zero form factor reference | 4–6 months | $95 (5k) | 57% |

**Smart move:** Launch a Flipper Zero companion app first. It's a sensor relay that Flipper owners can buy for $50 as a plug-in module. This builds the community before the General device ships. Then the General captures users who want an all-in-one.

### Macro — Raspberry Pi is already off-the-shelf

The Macro is essentially a Raspberry Pi with a sensor HAT and weatherproof enclosure. This is commercially available in parts today:

| Option | Time to market | Unit cost | Margin at $500 |
|--------|---------------|-----------|----------------|
| Custom carrier board + CM4 (recommended) | 4–6 months | $226 (1k) | 54.8% |
| Pi 5 + Pimoroni Enviro HAT + enclosure (kit) | **2–4 weeks** | $285 assembled | 43% |
| Pi Zero 2W + custom HAT | 3–4 months | $145 (1k) | 71% |

**The fast-to-market Macro:** A Raspberry Pi 5 (or Zero 2W) with off-the-shelf Pimoroni sensor HATs in an off-the-shelf weatherproof enclosure + 18650 battery pack. Total build time: weeks, not months. Lower margin but zero certification delay for the compute portion (Pi is already FCC/CE certified).

This is the right Year 1 Macro strategy: sell a kit that users assemble, using a standard Pi as the brain. Custom carrier board ships in Year 2 once the design is validated.

---

## Recommended Phased Manufacturing Plan

### Phase 1 — Developer Edition (Months 1–6)
*Goal: 500 units, prove the hardware, build community*

- **Micro**: Use Seeed XIAO nRF52840 + sensors on a simple breakout PCB. $55 BOM. Sell as a dev kit at $100, explicitly labeled "Developer Edition, not FCC certified."
- **General**: JLCPCB PCBA run of 200 units. $125 BOM. Sell at $180 developer price.
- **Macro**: Raspberry Pi 5 + Pimoroni HATs + Hammond enclosure + 18650 sled. Off-the-shelf kit. $285 BOM. Sell at $450 developer price.
- **Begin FCC/CE certification testing in parallel** — submit to lab in Month 2.
- **Target channels**: BTCPC website direct, Crowd Supply (hardware crowdfunding), Hackaday community.

### Phase 2 — Consumer Launch (Months 7–12)
*Goal: 5,000 units, certified hardware, Amazon listing*

- All three tiers with FCC/CE certification in hand.
- Seeed Studio OEM for General (2,000–3,000 unit run).
- JLCPCB for Micro (3,000–5,000 unit run).
- Begin Macro custom carrier board production (500–1,000 unit run).
- Amazon FBA for Micro and General. Direct for Macro.
- **Safety stock**: Pre-purchase 6-month supply of Sensirion sensors.

### Phase 3 — Scale (Year 2+)
*Goal: 44,000 units, Shenzhen EMS partner, ODM relationship*

- Shenzhen EMS for all three tiers.
- Negotiate direct with Sensirion for annual purchase agreement (reduces SCD40/SPS30 lead time to 4 weeks).
- Flipper Zero-style open hardware release — community builds drive organic demand.
- Introduce bundle pricing: "BTCPC Starter Pack" (1 General + 3 Micros, $450).

---

## Working Capital Requirements

To manufacture and hold 30 days of inventory at each phase:

| Phase | Monthly unit target | Inventory (30 days) | Safety stock (Sensirion) | **Total working capital** |
|-------|--------------------|--------------------|--------------------------|--------------------------|
| Phase 1 | 80 units | $18k | $0 (low volume) | **$25k** |
| Phase 2 | 400 units | $75k | $55k | **$140k** |
| Phase 3 | 3,600 units | $550k | $130k | **$700k** |

Phase 1 is bootstrppable with $25–30k. Phase 2 requires ~$150k in manufacturing capital. Phase 3 likely requires a credit line or raise.

---

## Key Risks and Mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| Sensirion sensor supply (SCD40, SPS30, SCD41) | High | 12-week safety stock; SGP41 as software-degraded fallback for CO2 |
| Raspberry Pi CM4 allocation | Medium | Approved reseller relationship (RS Components, OKdo); Radxa CM3 as drop-in alternative |
| FCC certification delay | High | Begin in Month 1; sell uncertified developer edition while cert is pending |
| Counterfeit components from gray market | Medium | Source only from DigiKey, Mouser, LCSC, or direct from manufacturer |
| Currency / tariff risk on Chinese manufacturing | Medium | US domestic EMS (MacroFab) for Phase 1 reduces exposure; dual-source strategy for Phase 2+ |
| Battery shipping (LiPo is hazmat) | Low | UN 38.3 certification covers air freight; standard for all consumer electronics |
