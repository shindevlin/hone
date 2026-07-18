#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..');
const baseAppDir = path.join(repoRoot, 'flipper', 'hone_sensor');
const variantsRoot = path.join(repoRoot, 'flipper', 'hone_sensor_variants');

const masks = {
  subghz: 'HONE_SENSOR_MASK_SUBGHZ',
  ble: 'HONE_SENSOR_MASK_BLE',
  nfc: 'HONE_SENSOR_MASK_NFC',
  gpio: 'HONE_SENSOR_MASK_GPIO',
  cpu_temp: 'HONE_SENSOR_MASK_CPU_TEMP',
  battery: 'HONE_SENSOR_MASK_BATTERY',
};

const profiles = [
  { id: 'auto_noop', label: 'auto-noop', sensors: [], autoExitOnly: true },
  { id: 'noop', label: 'noop', sensors: [] },
  { id: 'auto_subghz', label: 'auto-subghz', sensors: [], autoProbeTarget: 'HONE_MENU_SUBGHZ' },
  { id: 'auto_ble', label: 'auto-ble', sensors: [], autoProbeTarget: 'HONE_MENU_BLE' },
  { id: 'auto_nfc', label: 'auto-nfc', sensors: [], autoProbeTarget: 'HONE_MENU_NFC' },
  { id: 'auto_gpio', label: 'auto-gpio', sensors: [], autoProbeTarget: 'HONE_MENU_GPIO' },
  { id: 'auto_cpu_temp', label: 'auto-cpu-temp', sensors: [], autoProbeTarget: 'HONE_MENU_CPU_TEMP' },
  { id: 'auto_battery', label: 'auto-battery', sensors: [], autoProbeTarget: 'HONE_MENU_BATTERY' },
  { id: 'battery_nostore', label: 'battery-nostore', sensors: ['battery'], noStore: true },
  { id: 'cpu_temp_nostore', label: 'cpu-temp-nostore', sensors: ['cpu_temp'], noStore: true },
  { id: 'nfc_nostore', label: 'nfc-nostore', sensors: ['nfc'], noStore: true },
  { id: 'all', label: 'all', sensors: ['subghz', 'ble', 'nfc', 'gpio', 'cpu_temp', 'battery'] },
  { id: 'subghz', label: 'subghz', sensors: ['subghz'] },
  { id: 'ble', label: 'ble', sensors: ['ble'] },
  { id: 'nfc', label: 'nfc', sensors: ['nfc'] },
  { id: 'gpio', label: 'gpio', sensors: ['gpio'] },
  { id: 'cpu_temp', label: 'cpu-temp', sensors: ['cpu_temp'] },
  { id: 'battery', label: 'battery', sensors: ['battery'] },
  { id: 'rf_ble', label: 'rf+ble', sensors: ['subghz', 'ble'] },
  { id: 'rf_nfc', label: 'rf+nfc', sensors: ['subghz', 'nfc'] },
  { id: 'rf_gpio', label: 'rf+gpio', sensors: ['subghz', 'gpio'] },
  { id: 'rf_temp', label: 'rf+temp', sensors: ['subghz', 'cpu_temp'] },
  { id: 'rf_battery', label: 'rf+battery', sensors: ['subghz', 'battery'] },
  { id: 'ble_nfc', label: 'ble+nfc', sensors: ['ble', 'nfc'] },
  { id: 'ble_gpio', label: 'ble+gpio', sensors: ['ble', 'gpio'] },
  { id: 'ble_temp', label: 'ble+temp', sensors: ['ble', 'cpu_temp'] },
  { id: 'ble_battery', label: 'ble+battery', sensors: ['ble', 'battery'] },
  { id: 'nfc_gpio', label: 'nfc+gpio', sensors: ['nfc', 'gpio'] },
  { id: 'nfc_temp', label: 'nfc+temp', sensors: ['nfc', 'cpu_temp'] },
  { id: 'gpio_temp', label: 'gpio+temp', sensors: ['gpio', 'cpu_temp'] },
  { id: 'rf_ble_nfc', label: 'rf+ble+nfc', sensors: ['subghz', 'ble', 'nfc'] },
];

function maskExpr(sensorNames) {
  if (sensorNames.length === 0) {
    return '0';
  }
  return sensorNames.map((sensor) => masks[sensor]).join(' | ');
}

function ensureDir(dir) {
  fs.mkdirSync(dir, { recursive: true });
}

function writeIfChanged(filePath, contents) {
  const existing = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : null;
  if (existing !== contents) {
    fs.writeFileSync(filePath, contents);
  }
}

function makeManifest(profile) {
  const appId = `hone_sensor_${profile.id}`;
  const displayName = `HONE Sensor ${profile.label}`;
  return `App(
    appid="${appId}",
    name="${displayName}",
    apptype=FlipperAppType.EXTERNAL,
    entry_point="hone_sensor_app",
    stack_size=8 * 1024,
    fap_category="HONE",
    fap_description="HONE sensor profile: ${profile.label}",
)
`;
}

function makeWrapper(profile) {
  const label = profile.label.replace(/"/g, '\\"');
  const noStoreLine = profile.noStore ? '#define HONE_ENABLE_PERSISTENCE 0\n' : '';
  const autoExitLine = profile.autoExitOnly ? '#define HONE_AUTO_EXIT_ONLY 1\n' : '';
  const autoProbeLine = profile.autoProbeTarget
    ? `#define HONE_AUTO_PROBE_TARGET ${profile.autoProbeTarget}\n`
    : '';
  return `${noStoreLine}${autoExitLine}${autoProbeLine}#define HONE_SENSOR_PROFILE_NAME "${label}"
#define HONE_SENSOR_PROFILE_MASK (${maskExpr(profile.sensors)})
#include "../../hone_sensor/hone_sensor.c"
`;
}

ensureDir(variantsRoot);

for (const profile of profiles) {
  const variantDir = path.join(variantsRoot, profile.id);
  ensureDir(variantDir);
  writeIfChanged(path.join(variantDir, 'application.fam'), makeManifest(profile));
  writeIfChanged(path.join(variantDir, 'variant.c'), makeWrapper(profile));
}

console.log(`Generated ${profiles.length} Flipper sensor variants in ${variantsRoot}`);
