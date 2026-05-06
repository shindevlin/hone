#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..');
const baseAppDir = path.join(repoRoot, 'flipper', 'btcpc_sensor');
const variantsRoot = path.join(repoRoot, 'flipper', 'btcpc_sensor_variants');

const masks = {
  subghz: 'BTCPC_SENSOR_MASK_SUBGHZ',
  ble: 'BTCPC_SENSOR_MASK_BLE',
  nfc: 'BTCPC_SENSOR_MASK_NFC',
  gpio: 'BTCPC_SENSOR_MASK_GPIO',
  cpu_temp: 'BTCPC_SENSOR_MASK_CPU_TEMP',
  battery: 'BTCPC_SENSOR_MASK_BATTERY',
};

const profiles = [
  { id: 'auto_noop', label: 'auto-noop', sensors: [], autoExitOnly: true },
  { id: 'noop', label: 'noop', sensors: [] },
  { id: 'auto_subghz', label: 'auto-subghz', sensors: [], autoProbeTarget: 'BTCPC_MENU_SUBGHZ' },
  { id: 'auto_ble', label: 'auto-ble', sensors: [], autoProbeTarget: 'BTCPC_MENU_BLE' },
  { id: 'auto_nfc', label: 'auto-nfc', sensors: [], autoProbeTarget: 'BTCPC_MENU_NFC' },
  { id: 'auto_gpio', label: 'auto-gpio', sensors: [], autoProbeTarget: 'BTCPC_MENU_GPIO' },
  { id: 'auto_cpu_temp', label: 'auto-cpu-temp', sensors: [], autoProbeTarget: 'BTCPC_MENU_CPU_TEMP' },
  { id: 'auto_battery', label: 'auto-battery', sensors: [], autoProbeTarget: 'BTCPC_MENU_BATTERY' },
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
  const appId = `btcpc_sensor_${profile.id}`;
  const displayName = `BTCPC Sensor ${profile.label}`;
  return `App(
    appid="${appId}",
    name="${displayName}",
    apptype=FlipperAppType.EXTERNAL,
    entry_point="btcpc_sensor_app",
    stack_size=8 * 1024,
    fap_category="BTCPC",
    fap_description="BTCPC sensor profile: ${profile.label}",
)
`;
}

function makeWrapper(profile) {
  const label = profile.label.replace(/"/g, '\\"');
  const noStoreLine = profile.noStore ? '#define BTCPC_ENABLE_PERSISTENCE 0\n' : '';
  const autoExitLine = profile.autoExitOnly ? '#define BTCPC_AUTO_EXIT_ONLY 1\n' : '';
  const autoProbeLine = profile.autoProbeTarget
    ? `#define BTCPC_AUTO_PROBE_TARGET ${profile.autoProbeTarget}\n`
    : '';
  return `${noStoreLine}${autoExitLine}${autoProbeLine}#define BTCPC_SENSOR_PROFILE_NAME "${label}"
#define BTCPC_SENSOR_PROFILE_MASK (${maskExpr(profile.sensors)})
#include "../../btcpc_sensor/btcpc_sensor.c"
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
