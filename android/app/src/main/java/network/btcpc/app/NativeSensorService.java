package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothManager;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanFilter;
import android.bluetooth.le.ScanRecord;
import android.bluetooth.le.ScanResult;
import android.bluetooth.le.ScanSettings;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.BroadcastReceiver;
import android.content.IntentFilter;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.net.wifi.WifiInfo;
import android.net.wifi.WifiManager;
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;
import android.os.BatteryManager;
import android.os.Build;
import android.os.IBinder;
import android.provider.Settings;
import android.util.Log;

import androidx.core.app.NotificationCompat;

import org.json.JSONObject;

import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

import net.i2p.crypto.eddsa.EdDSAEngine;
import net.i2p.crypto.eddsa.EdDSAPrivateKey;
import net.i2p.crypto.eddsa.EdDSAPublicKey;
import net.i2p.crypto.eddsa.spec.EdDSANamedCurveTable;
import net.i2p.crypto.eddsa.spec.EdDSAParameterSpec;
import net.i2p.crypto.eddsa.spec.EdDSAPrivateKeySpec;
import net.i2p.crypto.eddsa.spec.EdDSAPublicKeySpec;

import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;

public class NativeSensorService extends Service implements SensorEventListener, LocationListener {

    static {
        System.loadLibrary("btcpc_miner");
    }

    private static native boolean nativeSubmitReading(
            String sensorId, String sensorType,
            double primaryValue, String valuesJson, String unit);

    private static final String TAG = "BTCPCSensor";
    private static final String CHANNEL_ID = "btcpc_sensors";
    private static final int NOTIFICATION_ID = 9440;
    private static final String PREFS = "btcpc_native_state";
    private volatile String apiBase = AppPrefs.DEFAULT_API_URL + "/api";
    private static final MediaType JSON = MediaType.get("application/json; charset=utf-8");

    private final OkHttpClient client = new OkHttpClient.Builder().retryOnConnectionFailure(true).build();
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private final Map<String, SensorSnapshot> snapshots = Collections.synchronizedMap(new HashMap<>());
    private final Set<String> registeredSensors = Collections.synchronizedSet(new HashSet<>());
    private final Set<String> keyRegisteredSensors = Collections.synchronizedSet(new HashSet<>());
    private final Map<String, SensorSigner> signers = Collections.synchronizedMap(new HashMap<>());
    private volatile boolean running;
    private volatile String account;
    private volatile String deviceName;
    private volatile String jwt;
    private volatile String postingKey;
    private SensorManager sensorManager;
    private LocationManager locationManager;
    private ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;
    private BroadcastReceiver batteryReceiver;
    private volatile boolean gpsUpdatesActive;

    // ── BLE tracker scanner ───────────────────────────────────────────────────
    // Apple company ID 0x004C, Find My marker bytes 0x12 / 0x1E
    private static final int APPLE_COMPANY_ID   = 0x004C;
    private static final int GOOGLE_COMPANY_ID  = 0x00E0;
    private static final int SAMSUNG_COMPANY_ID = 0x0075;
    // Tile service UUID 0xFEED → parcelUuid "0000feed-0000-1000-8000-00805f9b34fb"
    private static final String UUID_TILE        = "0000feed-0000-1000-8000-00805f9b34fb";
    private static final String UUID_AIRTAG      = "0000fd6f-0000-1000-8000-00805f9b34fb";
    private static final String UUID_ANDROID_FMD = "0000fce0-0000-1000-8000-00805f9b34fb";
    private static final String UUID_SAMSUNG     = "0000fd5a-0000-1000-8000-00805f9b34fb";
    private static final int MIN_RSSI = -90; // dBm

    private BluetoothLeScanner bleScanner;
    private ScanCallback bleScanCallback;
    // Sighting counts for current epoch, reset each flush.
    private final java.util.concurrent.atomic.AtomicInteger airtagCount   = new java.util.concurrent.atomic.AtomicInteger(0);
    private final java.util.concurrent.atomic.AtomicInteger androidFmdCount = new java.util.concurrent.atomic.AtomicInteger(0);
    private final java.util.concurrent.atomic.AtomicInteger tileCount     = new java.util.concurrent.atomic.AtomicInteger(0);
    private final java.util.concurrent.atomic.AtomicInteger samsungCount  = new java.util.concurrent.atomic.AtomicInteger(0);
    private final java.util.concurrent.atomic.AtomicInteger otherCount    = new java.util.concurrent.atomic.AtomicInteger(0);
    // Per-epoch commitment hashes (for batch_hash).
    private final java.util.concurrent.CopyOnWriteArrayList<String> sightingCommitments =
            new java.util.concurrent.CopyOnWriteArrayList<>();

    @Override
    public void onCreate() {
        super.onCreate();
        sensorManager   = (SensorManager) getSystemService(Context.SENSOR_SERVICE);
        locationManager = (LocationManager) getSystemService(Context.LOCATION_SERVICE);
        connectivityManager = (ConnectivityManager) getSystemService(Context.CONNECTIVITY_SERVICE);
        createNotificationChannel();
        loadSettings();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        loadSettings();
        running = true;

        Notification notification = buildNotification("Phone sensors: collecting");
        try {
            startForeground(NOTIFICATION_ID, notification);
        } catch (Exception e) {
            android.util.Log.w(TAG, "startForeground failed: " + e.getMessage());
            stopSelf();
            return START_NOT_STICKY;
        }
        registerSensors();
        registerNetworkMonitor();
        applyGpsPolicy();
        registerBatteryReceiver();
        startBleTrackerScan();
        scheduler.scheduleAtFixedRate(this::flushSnapshotsSafe, 10, 30, TimeUnit.SECONDS);
        persistState("Sensors: running");
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        running = false;
        stopBleTrackerScan();
        scheduler.shutdownNow();
        io.shutdownNow();
        if (sensorManager != null) sensorManager.unregisterListener(this);
        stopLocationUpdates();
        unregisterNetworkMonitor();
        if (batteryReceiver != null) {
            try { unregisterReceiver(batteryReceiver); } catch (IllegalArgumentException ignored) {}
        }
        persistState("Sensors: stopped");
        super.onDestroy();
    }

    // ── BLE tracker scanning ──────────────────────────────────────────────────

    private void startBleTrackerScan() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (checkSelfPermission(android.Manifest.permission.BLUETOOTH_SCAN)
                    != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                Log.w(TAG, "BLUETOOTH_SCAN not granted — BLE tracker scan skipped");
                return;
            }
        }
        BluetoothManager btMgr = (BluetoothManager) getSystemService(Context.BLUETOOTH_SERVICE);
        if (btMgr == null) return;
        BluetoothAdapter adapter = btMgr.getAdapter();
        if (adapter == null || !adapter.isEnabled()) {
            Log.w(TAG, "BLE not available, tracker scan skipped");
            return;
        }
        bleScanner = adapter.getBluetoothLeScanner();
        if (bleScanner == null) return;

        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_POWER)
                .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
                .build();

        bleScanCallback = new ScanCallback() {
            @Override
            public void onScanResult(int callbackType, ScanResult result) {
                if (!running) return;
                int rssi = result.getRssi();
                if (rssi < MIN_RSSI) return;
                ScanRecord rec = result.getScanRecord();
                if (rec == null) return;

                String tagType = classifyBleTag(rec);
                if (tagType == null) return;

                // Privacy commitment: H(epochWindow || accountBytes || macBytes)
                String mac = result.getDevice().getAddress();
                long epochWindow = System.currentTimeMillis() / 1000L / 30L / 30L; // ~15-min windows
                String commitment = sha256Hex(epochWindow + account + mac);
                sightingCommitments.add(commitment);

                switch (tagType) {
                    case "AirTag":     airtagCount.incrementAndGet();   break;
                    case "AndroidFMD": androidFmdCount.incrementAndGet(); break;
                    case "Tile":       tileCount.incrementAndGet();     break;
                    case "Samsung":    samsungCount.incrementAndGet();  break;
                    default:           otherCount.incrementAndGet();    break;
                }
                Log.d(TAG, "BLE tracker: " + tagType + " rssi=" + rssi);
            }

            @Override
            public void onScanFailed(int errorCode) {
                Log.w(TAG, "BLE scan failed: " + errorCode);
            }
        };

        bleScanner.startScan(null, settings, bleScanCallback);
        Log.i(TAG, "BLE tracker scan started");
    }

    private void stopBleTrackerScan() {
        if (bleScanner != null && bleScanCallback != null) {
            try { bleScanner.stopScan(bleScanCallback); } catch (Exception ignored) {}
        }
    }

    /** Classify a BLE advertisement by manufacturer data + service UUIDs. */
    private String classifyBleTag(ScanRecord rec) {
        // Apple check (manufacturer data, company ID 0x004C)
        byte[] appleData = rec.getManufacturerSpecificData(APPLE_COMPANY_ID);
        if (appleData != null && appleData.length >= 2) {
            int type = appleData[0] & 0xFF;
            if (type == 0x12 || type == 0x1E) return "AirTag";
        }

        // Google Find My Device
        if (rec.getManufacturerSpecificData(GOOGLE_COMPANY_ID) != null) return "AndroidFMD";

        // Samsung SmartTag
        byte[] samsungData = rec.getManufacturerSpecificData(SAMSUNG_COMPANY_ID);
        if (samsungData != null && samsungData.length >= 4) return "Samsung";

        // Service UUID checks
        List<android.os.ParcelUuid> uuids = rec.getServiceUuids();
        if (uuids != null) {
            for (android.os.ParcelUuid uuid : uuids) {
                String s = uuid.toString().toLowerCase(Locale.ROOT);
                if (s.equals(UUID_AIRTAG))      return "AirTag";
                if (s.equals(UUID_ANDROID_FMD)) return "AndroidFMD";
                if (s.equals(UUID_TILE))        return "Tile";
                if (s.equals(UUID_SAMSUNG))     return "Samsung";
            }
        }
        return null;
    }

    /** Flush BLE sighting batch to the node API, then reset counters. */
    private void flushBleTrackerBatch(long epoch) {
        int at = airtagCount.getAndSet(0);
        int af = androidFmdCount.getAndSet(0);
        int ti = tileCount.getAndSet(0);
        int sa = samsungCount.getAndSet(0);
        int ot = otherCount.getAndSet(0);
        if (at + af + ti + sa + ot == 0) return;

        java.util.ArrayList<String> commits = new java.util.ArrayList<>(sightingCommitments);
        sightingCommitments.clear();
        java.util.Collections.sort(commits);
        String batchHash = sha256Hex(String.join("|", commits));
        String observerId = "android-" + account + "/ble";

        try {
            JSONObject body = new JSONObject();
            body.put("observer_id", observerId);
            body.put("owner", account);
            body.put("airtag_count", at);
            body.put("android_fmd_count", af);
            body.put("tile_count", ti);
            body.put("samsung_count", sa);
            body.put("other_count", ot);
            body.put("batch_hash", batchHash);
            body.put("epoch", epoch);
            body.put("signed_by", account);
            postJson(apiBase + "/tracker/sighting", body);
            Log.i(TAG, "BLE batch committed: airtag=" + at + " fmd=" + af + " tile=" + ti);
        } catch (Exception e) {
            Log.w(TAG, "BLE batch commit failed: " + e.getMessage());
        }
    }

    private static String sha256Hex(String input) {
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            byte[] hash = md.digest(input.getBytes(StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder();
            for (byte b : hash) sb.append(String.format("%02x", b));
            return sb.toString();
        } catch (Exception e) {
            return "";
        }
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public void onSensorChanged(SensorEvent event) {
        if (!running || event == null || event.sensor == null) return;
        String sensorType = mapSensorType(event.sensor.getType());
        if (sensorType == null) return;
        String sensorId = account + "/" + deviceName + "-" + sensorType;
        SensorSnapshot snapshot = new SensorSnapshot();
        snapshot.sensorId = sensorId;
        snapshot.type = sensorType;
        snapshot.unit = sensorUnit(event.sensor.getType());
        snapshot.timestamp = System.currentTimeMillis();
        snapshot.values = copyValues(event.values);
        snapshot.androidType = event.sensor.getType();
        snapshot.name = event.sensor.getName();
        snapshot.vendor = event.sensor.getVendor();
        snapshots.put(sensorType, snapshot);
        persistState("Sensors active: " + snapshots.size());
    }

    @Override
    public void onAccuracyChanged(Sensor sensor, int accuracy) {
        // no-op
    }

    private void registerSensors() {
        if (sensorManager == null) return;
        sensorManager.unregisterListener(this);
        registeredSensors.clear();
        AppPrefs prefs = new AppPrefs(this);
        List<Sensor> all = sensorManager.getSensorList(Sensor.TYPE_ALL);
        for (Sensor sensor : all) {
            if (!isSensorGroupEnabled(prefs, sensor.getType())) continue;
            sensorManager.registerListener(this, sensor, SensorManager.SENSOR_DELAY_NORMAL);
        }
        persistState("Sensors: listeners registered");
    }

    private boolean isSensorGroupEnabled(AppPrefs prefs, int type) {
        switch (type) {
            // ── Motion ────────────────────────────────────────────────────────
            case Sensor.TYPE_ACCELEROMETER:
            case Sensor.TYPE_LINEAR_ACCELERATION:
            case Sensor.TYPE_GRAVITY:
            case TYPE_GYROSCOPE_UNCALIBRATED:
            case TYPE_ACCELEROMETER_UNCALIBRATED:
            case Sensor.TYPE_GYROSCOPE:
            case TYPE_SIGNIFICANT_MOTION:
            case TYPE_TILT_DETECTOR:
            case TYPE_STATIONARY_DETECT:
            case TYPE_MOTION_DETECT:
            case TYPE_WAKE_GESTURE:
            case TYPE_GLANCE_GESTURE:
            case TYPE_PICK_UP_GESTURE:
            case TYPE_WRIST_TILT_GESTURE:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MOTION);
            // ── Orientation ───────────────────────────────────────────────────
            case Sensor.TYPE_ROTATION_VECTOR:
            case TYPE_GAME_ROTATION_VECTOR:
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:
            case TYPE_DEVICE_ORIENTATION:
            case TYPE_HEADING:
            case TYPE_POSE_6DOF:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ORIENTATION);
            // ── Magnetometer ──────────────────────────────────────────────────
            case Sensor.TYPE_MAGNETIC_FIELD:
            case TYPE_MAGNETIC_FIELD_UNCALIBRATED:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MAGNETOMETER);
            // ── Environment ───────────────────────────────────────────────────
            case Sensor.TYPE_LIGHT:
            case Sensor.TYPE_PRESSURE:
            case TYPE_AMBIENT_TEMPERATURE:
            case TYPE_RELATIVE_HUMIDITY:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ENVIRONMENT);
            // ── Proximity / physical ──────────────────────────────────────────
            case Sensor.TYPE_PROXIMITY:
            case TYPE_HINGE_ANGLE:
            case TYPE_LOW_LATENCY_OFFBODY_DETECT:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_PROXIMITY);
            // ── Activity / step ───────────────────────────────────────────────
            case Sensor.TYPE_STEP_COUNTER:
            case Sensor.TYPE_STEP_DETECTOR:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_STEPS);
            // ── Biometric ─────────────────────────────────────────────────────
            case Sensor.TYPE_HEART_RATE:
            case TYPE_HEART_BEAT:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_HEARTRATE);
            // ── Everything else: allow if any sensor group enabled ────────────
            default:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MOTION)
                    || prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ENVIRONMENT);
        }
    }

    private void registerNetworkMonitor() {
        if (connectivityManager == null || networkCallback != null) return;
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override
            public void onAvailable(Network network) {
                applyGpsPolicy();
            }

            @Override
            public void onLost(Network network) {
                applyGpsPolicy();
            }

            @Override
            public void onCapabilitiesChanged(Network network, NetworkCapabilities capabilities) {
                applyGpsPolicy();
            }
        };
        try {
            connectivityManager.registerDefaultNetworkCallback(networkCallback);
        } catch (Exception e) {
            Log.w(TAG, "network monitor failed: " + e.getMessage());
        }
    }

    private void unregisterNetworkMonitor() {
        if (connectivityManager == null || networkCallback == null) return;
        try {
            connectivityManager.unregisterNetworkCallback(networkCallback);
        } catch (Exception ignored) {}
        networkCallback = null;
    }

    private void applyGpsPolicy() {
        if (locationManager == null) return;
        AppPrefs prefs = new AppPrefs(this);
        boolean gpsEnabled = prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_GPS);
        boolean trustedWifi = isOnTrustedWifi(prefs);
        boolean shouldRun = gpsEnabled && !trustedWifi;
        try {
            stopLocationUpdates();
            if (!shouldRun) {
                persistState(trustedWifi ? "GPS paused on trusted Wi-Fi" : "GPS paused");
                return;
            }
            if (checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION)
                    != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                persistState("GPS waiting for permission");
                return;
            }
            locationManager.requestLocationUpdates(
                    LocationManager.GPS_PROVIDER, 60_000L, 50f, this);
            locationManager.requestLocationUpdates(
                    LocationManager.PASSIVE_PROVIDER, 120_000L, 100f, this);
            gpsUpdatesActive = true;
            persistState("GPS active");
        } catch (SecurityException e) {
            Log.w(TAG, "GPS permission denied: " + e.getMessage());
        } catch (Exception e) {
            Log.w(TAG, "GPS registration failed: " + e.getMessage());
        }
    }

    private void stopLocationUpdates() {
        if (locationManager == null) return;
        try { locationManager.removeUpdates(this); } catch (SecurityException ignored) {}
        gpsUpdatesActive = false;
    }

    private boolean isOnTrustedWifi(AppPrefs prefs) {
        if (connectivityManager == null) return false;

        Network active = connectivityManager.getActiveNetwork();
        if (active == null) return false;

        NetworkCapabilities capabilities = connectivityManager.getNetworkCapabilities(active);
        if (capabilities == null || !capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
            return false;
        }

        String ssid = currentWifiSsid();
        return TrustedWifiPolicy.matches(ssid, prefs.getTrustedWifiSsidSet());
    }

    private String currentWifiSsid() {
        try {
            WifiManager wifiManager = (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wifiManager == null) return "";
            WifiInfo info = wifiManager.getConnectionInfo();
            if (info == null) return "";
            return TrustedWifiPolicy.normalizeSsid(info.getSSID());
        } catch (Exception e) {
            Log.w(TAG, "wifi ssid lookup failed: " + e.getMessage());
            return "";
        }
    }

    private void registerBatteryReceiver() {
        if (!new AppPrefs(this).isSensorEnabled(AppPrefs.KEY_SENSOR_BATTERY)) return;
        batteryReceiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                try {
                    int level       = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
                    int scale       = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100);
                    int status      = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1);
                    int voltage_mv  = intent.getIntExtra(BatteryManager.EXTRA_VOLTAGE, 0);
                    int temperature = intent.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, 0);
                    if (level < 0 || scale <= 0) return;
                    double pct = (level * 100.0) / scale;
                    boolean charging = status == BatteryManager.BATTERY_STATUS_CHARGING
                            || status == BatteryManager.BATTERY_STATUS_FULL;

                    SensorSnapshot snap = new SensorSnapshot();
                    snap.type = "battery";
                    snap.unit = "%";
                    snap.sensorId = account + "/" + deviceName + "-battery";
                    snap.timestamp = System.currentTimeMillis();
                    // values: [pct, voltage_mv, temp_tenths_celsius, charging(1/0)]
                    snap.values = new double[]{pct, voltage_mv, temperature, charging ? 1 : 0};
                    snap.androidType = -1;
                    snap.name = "Battery";
                    snap.vendor = Build.MANUFACTURER;
                    snapshots.put("battery", snap);
                } catch (Exception e) {
                    Log.w(TAG, "Battery receiver error: " + e.getMessage());
                }
            }
        };
        IntentFilter filter = new IntentFilter(Intent.ACTION_BATTERY_CHANGED);
        registerReceiver(batteryReceiver, filter);
        // Trigger an immediate reading
        Intent sticky = registerReceiver(null, filter);
        if (sticky != null) batteryReceiver.onReceive(this, sticky);
    }

    private void refreshWifiReading() {
        try {
            WifiManager wm = (WifiManager) getApplicationContext()
                    .getSystemService(Context.WIFI_SERVICE);
            if (wm == null || !wm.isWifiEnabled()) return;
            android.net.wifi.WifiInfo info = wm.getConnectionInfo();
            if (info == null) return;
            int rssi = info.getRssi();
            int linkSpeed = info.getLinkSpeed(); // Mbps
            if (rssi <= -127) return;

            SensorSnapshot snap = new SensorSnapshot();
            snap.type = "wifi-rssi";
            snap.unit = "dBm";
            snap.sensorId = account + "/" + deviceName + "-wifi-rssi";
            snap.timestamp = System.currentTimeMillis();
            snap.values = new double[]{rssi, linkSpeed};
            snap.androidType = -3;
            snap.name = "WiFi";
            snap.vendor = "Android";
            snapshots.put("wifi-rssi", snap);
        } catch (Exception e) {
            Log.w(TAG, "WiFi reading error: " + e.getMessage());
        }
    }

    // LocationListener implementation
    @Override
    public void onLocationChanged(Location location) {
        if (!running || location == null) return;
        SensorSnapshot snap = new SensorSnapshot();
        snap.type = "gps-location";
        snap.unit = "deg";
        snap.sensorId = account + "/" + deviceName + "-gps-location";
        snap.timestamp = location.getTime();
        snap.values = new double[]{
                location.getLatitude(), location.getLongitude(),
                location.getAltitude(), location.getAccuracy(),
                location.getSpeed(), location.getBearing()};
        snap.androidType = -2;
        snap.name = "GPS";
        snap.vendor = "Android";
        snapshots.put("gps-location", snap);
        persistState("GPS fix: ±" + (int) location.getAccuracy() + "m");
    }

    @Override public void onStatusChanged(String provider, int status, android.os.Bundle extras) {}
    @Override public void onProviderEnabled(String provider) {}
    @Override public void onProviderDisabled(String provider) {}

    private void flushSnapshotsSafe() {
        if (!running) return;
        refreshWifiReading(); // pull latest WiFi RSSI before flushing
        io.submit(() -> {
            // Snapshot under lock, then release before network I/O so onSensorChanged
            // (main thread) never blocks waiting for a network call to complete.
            List<SensorSnapshot> toFlush;
            synchronized (snapshots) {
                toFlush = new java.util.ArrayList<>(snapshots.values());
            }
            for (SensorSnapshot snapshot : toFlush) {
                if (snapshot == null || snapshot.values == null || snapshot.values.length == 0) continue;
                ensureRegistered(snapshot);
                ensureKeyRegistered(snapshot);
                submitReading(snapshot);
            }
            // Flush BLE tracker batch alongside sensor snapshots.
            long epoch = System.currentTimeMillis() / 1000L / 30L;
            flushBleTrackerBatch(epoch);
        });
    }

    private void ensureRegistered(SensorSnapshot snapshot) {
        if (registeredSensors.contains(snapshot.sensorId)) return;
        try {
            String sensorId = snapshot.sensorId;
            JSONObject body = new JSONObject();
            body.put("account", account);
            body.put("name", deviceName + "-" + snapshot.type);
            body.put("type", snapshot.type);
            body.put("unit", snapshot.unit);
            body.put("decimals", 6);
            body.put("region", "phone");
            body.put("hardware_model", Build.MODEL);
            body.put("firmware_version", Build.VERSION.RELEASE);
            body.put("allow_precise_location", false);
            SensorSigner regSigner = getSigner(sensorId);
            if (regSigner != null) body.put("public_key", regSigner.publicKeySpkiHex);
            postJson(apiBase + "/sensors", body);
            registeredSensors.add(sensorId);
            persistState("Sensors: registered " + sensorId);
        } catch (Exception e) {
            Log.w(TAG, "register failed: " + e.getMessage());
        }
    }

    private void ensureKeyRegistered(SensorSnapshot snapshot) {
        String sensorId = snapshot.sensorId;
        if (keyRegisteredSensors.contains(sensorId)) return;
        if (jwt == null || jwt.isEmpty()) {
            // Register-key requires owner JWT; skip silently until user logs in
            keyRegisteredSensors.add(sensorId);
            return;
        }
        SensorSigner signer = getSigner(sensorId);
        if (signer == null) { keyRegisteredSensors.add(sensorId); return; }
        try {
            JSONObject body = new JSONObject();
            body.put("public_key", signer.publicKeySpkiHex);
            postJson(apiBase + "/sensors/" + encode(sensorId) + "/register-key", body);
            keyRegisteredSensors.add(sensorId);
            Log.i(TAG, "Sensor key registered: " + sensorId);
        } catch (Exception e) {
            Log.w(TAG, "key-register failed (will retry next session): " + e.getMessage());
            // Don't add to keyRegisteredSensors — will retry on next service start
            keyRegisteredSensors.add(sensorId);
        }
    }

    private SensorSigner getSigner(String sensorId) {
        if (postingKey == null || postingKey.length() != 64) return null;
        SensorSigner existing = signers.get(sensorId);
        if (existing != null) return existing;
        try {
            SensorSigner s = new SensorSigner(postingKey, deviceName, sensorId);
            signers.put(sensorId, s);
            return s;
        } catch (Exception e) {
            Log.w(TAG, "SensorSigner init failed: " + e.getMessage());
            return null;
        }
    }

    private void submitReading(SensorSnapshot snapshot) {
        try {
            double value = normalizeValue(snapshot);
            String valuesJson = buildValuesJson(snapshot);

            // Submit to local Rust micronode first.
            try {
                nativeSubmitReading(snapshot.sensorId, snapshot.type, value, valuesJson, snapshot.unit);
            } catch (UnsatisfiedLinkError e) {
                Log.w(TAG, "nativeSubmitReading unavailable: " + e.getMessage());
            }

            // Also POST to remote API for network-wide propagation.
            JSONObject metadata = new JSONObject();
            metadata.put("type", snapshot.type);
            metadata.put("unit", snapshot.unit);
            metadata.put("source", "android-native");
            metadata.put("android_type", snapshot.androidType);
            metadata.put("name", snapshot.name);
            metadata.put("vendor", snapshot.vendor);
            metadata.put("raw_values", snapshot.valuesToJsonArray());

            JSONObject body = new JSONObject();
            body.put("account", account);
            body.put("value", value);
            body.put("metadata", metadata);

            long deviceTimestamp = snapshot.timestamp;
            SensorSigner signer = getSigner(snapshot.sensorId);
            if (signer != null) {
                try {
                    body.put("reading_sig", signer.sign(value, deviceTimestamp));
                    body.put("device_timestamp", deviceTimestamp);
                } catch (Exception e) {
                    Log.w(TAG, "sign failed: " + e.getMessage());
                }
            }

            String url = apiBase + "/sensors/" + encode(snapshot.sensorId) + "/readings";
            postJson(url, body);
            persistState(String.format(Locale.US, "Sensors: submitted %s=%.3f", snapshot.type, value));
        } catch (Exception e) {
            Log.w(TAG, "submit failed: " + e.getMessage());
        }
    }

    private static String buildValuesJson(SensorSnapshot s) {
        try {
            double[] v = s.values;
            JSONObject j = new JSONObject();
            switch (s.type) {
                // ── 3-axis vectors ────────────────────────────────────────────
                case "accelerometer":
                case "linear-acceleration":
                case "gravity":
                case "gyroscope":
                case "magnetometer":
                    if (v.length >= 3) { j.put("x", v[0]); j.put("y", v[1]); j.put("z", v[2]); }
                    break;
                // ── 3-axis + 3-axis bias (uncalibrated) ──────────────────────
                case "accelerometer-raw":
                case "gyroscope-raw":
                case "magnetometer-raw":
                    if (v.length >= 3) { j.put("x", v[0]); j.put("y", v[1]); j.put("z", v[2]); }
                    if (v.length >= 6) { j.put("bx", v[3]); j.put("by", v[4]); j.put("bz", v[5]); }
                    break;
                // ── Quaternion orientation ────────────────────────────────────
                case "orientation":
                case "orientation-game":
                case "orientation-geo":
                    if (v.length >= 4) {
                        j.put("qx", v[0]); j.put("qy", v[1]);
                        j.put("qz", v[2]); j.put("qw", v[3]);
                    } else if (v.length >= 3) {
                        j.put("azimuth", v[0]); j.put("pitch", v[1]); j.put("roll", v[2]);
                    }
                    break;
                // ── 6DOF pose ─────────────────────────────────────────────────
                case "pose-6dof":
                    if (v.length >= 7) {
                        j.put("qx", v[0]); j.put("qy", v[1]); j.put("qz", v[2]); j.put("qw", v[3]);
                        j.put("tx", v[4]); j.put("ty", v[5]); j.put("tz", v[6]);
                    }
                    break;
                // ── Device orientation (0=portrait, 1=landscape-ccw, 2=landscape-cw, 3=reverse) ──
                case "device-orientation":
                    if (v.length >= 1) j.put("rotation", (int) v[0]);
                    break;
                // ── Environmental ─────────────────────────────────────────────
                case "light":
                    if (v.length >= 1) j.put("lux", v[0]);
                    break;
                case "barometer":
                    if (v.length >= 1) j.put("hPa", v[0]);
                    break;
                case "humidity":
                    if (v.length >= 1) j.put("pct", v[0]);
                    break;
                case "temperature":
                    if (v.length >= 1) j.put("celsius", v[0]);
                    break;
                // ── Proximity / physical ──────────────────────────────────────
                case "proximity":
                    if (v.length >= 1) j.put("cm", v[0]);
                    break;
                case "hinge-angle":
                    if (v.length >= 1) j.put("degrees", v[0]);
                    break;
                case "heading":
                    if (v.length >= 1) j.put("degrees", v[0]);
                    if (v.length >= 2) j.put("accuracy", v[1]);
                    break;
                case "off-body":
                    if (v.length >= 1) j.put("on_body", v[0] > 0.5);
                    break;
                // ── Activity / step ───────────────────────────────────────────
                case "steps":
                    if (v.length >= 1) j.put("count", (long) v[0]);
                    break;
                case "step-detector":
                    if (v.length >= 1) j.put("step", v[0]);
                    break;
                // ── Biometric ─────────────────────────────────────────────────
                case "heart-rate":
                    if (v.length >= 1) j.put("bpm", v[0]);
                    break;
                case "heart-beat":
                    if (v.length >= 1) j.put("confidence", v[0]);
                    break;
                // ── Trigger / event sensors ───────────────────────────────────
                case "significant-motion":
                case "tilt":
                case "stationary":
                case "motion":
                case "wake-gesture":
                case "glance-gesture":
                case "pick-up":
                case "wrist-tilt":
                    j.put("event", 1);
                    break;
                // ── Virtual sensors (non-SensorManager) ──────────────────────
                case "gps-location":
                    if (v.length >= 2) { j.put("lat", v[0]); j.put("lon", v[1]); }
                    if (v.length >= 3) j.put("alt", v[2]);
                    if (v.length >= 4) j.put("accuracy_m", v[3]);
                    if (v.length >= 5) j.put("speed_ms", v[4]);
                    if (v.length >= 6) j.put("bearing_deg", v[5]);
                    break;
                case "battery":
                    if (v.length >= 1) j.put("pct", v[0]);
                    if (v.length >= 2) j.put("voltage_mv", v[1]);
                    if (v.length >= 3) j.put("temp_c", v[2] / 10.0);
                    if (v.length >= 4) j.put("charging", v[3] > 0);
                    break;
                case "wifi-rssi":
                    if (v.length >= 1) j.put("rssi_dbm", v[0]);
                    if (v.length >= 2) j.put("link_speed_mbps", v[1]);
                    break;
                case "cell-signal":
                    if (v.length >= 1) j.put("rssi_dbm", v[0]);
                    if (v.length >= 2) j.put("type", (int) v[1]);
                    break;
                case "audio-level":
                    if (v.length >= 1) j.put("db", v[0]);
                    break;
                default:
                    // Unknown sensor — dump all values
                    for (int i = 0; i < v.length; i++) j.put("v" + i, v[i]);
                    break;
            }
            return j.toString();
        } catch (Exception e) {
            return "{}";
        }
    }

    private void postJson(String url, JSONObject body) throws Exception {
        Request.Builder rb = new Request.Builder()
                .url(url)
                .post(RequestBody.create(body.toString(), JSON));
        if (jwt != null && !jwt.isEmpty()) rb.addHeader("Authorization", "Bearer " + jwt);
        Request request = rb.build();
        try (Response response = client.newCall(request).execute()) {
            if (!response.isSuccessful()) {
                String errBody = response.body() != null ? response.body().string() : "";
                throw new IllegalStateException("HTTP " + response.code() + " " + errBody);
            }
        }
    }

    private void loadSettings() {
        SharedPreferences prefs = getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        account = prefs.getString("account", "btcpc-phone");
        jwt     = prefs.getString("jwt", "");

        String newPostingKey = new AppPrefs(this).getPostingKey();
        if (!newPostingKey.equals(postingKey != null ? postingKey : "")) {
            // Posting key changed — invalidate cached signers so keys are rederived
            signers.clear();
            keyRegisteredSensors.clear();
        }
        postingKey = newPostingKey;
        apiBase = new AppPrefs(this).getEffectiveApiUrl() + "/api";

        String saved = prefs.getString("sensor_device_name", null);
        if (saved == null || saved.isEmpty()) {
            saved = sanitize(Settings.Secure.getString(getContentResolver(), Settings.Secure.ANDROID_ID));
            if (saved == null || saved.isEmpty()) {
                saved = "android";
            }
            prefs.edit().putString("sensor_device_name", saved).apply();
        }
        deviceName = saved;
    }

    private void persistState(String state) {
        SharedPreferences prefs = getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        prefs.edit()
                .putString("sensor_state", state)
                .putString("last_event", state)
                .apply();
    }

    private Notification buildNotification(String text) {
        Intent intent = new Intent(this, MainActivity.class);
        PendingIntent pendingIntent = PendingIntent.getActivity(
                this,
                0,
                intent,
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.M ? PendingIntent.FLAG_IMMUTABLE : 0
        );

        return new NotificationCompat.Builder(this, CHANNEL_ID)
                .setContentTitle("BTCPC Sensors")
                .setContentText(text)
                .setSmallIcon(android.R.drawable.ic_menu_compass)
                .setContentIntent(pendingIntent)
                .setOngoing(true)
                .build();
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "BTCPC Sensors",
                    NotificationManager.IMPORTANCE_LOW
            );
            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) manager.createNotificationChannel(channel);
        }
    }

    private static String encode(String value) {
        return java.net.URLEncoder.encode(value, java.nio.charset.StandardCharsets.UTF_8);
    }

    private static String sanitize(String value) {
        if (value == null) return null;
        return value.replaceAll("[^a-zA-Z0-9._-]", "").toLowerCase(Locale.US);
    }

    // Numeric constants for sensor types added in later API levels
    // (avoid @RequiresApi by using the raw int values directly)
    // ── Android sensor type constants not in older SDK levels ────────────────
    private static final int TYPE_RELATIVE_HUMIDITY              = 12;
    private static final int TYPE_AMBIENT_TEMPERATURE            = 13;
    private static final int TYPE_MAGNETIC_FIELD_UNCALIBRATED    = 14;
    private static final int TYPE_GAME_ROTATION_VECTOR           = 15;
    private static final int TYPE_GYROSCOPE_UNCALIBRATED         = 16;
    private static final int TYPE_SIGNIFICANT_MOTION             = 17;
    private static final int TYPE_TILT_DETECTOR                  = 22;
    private static final int TYPE_WAKE_GESTURE                   = 23;
    private static final int TYPE_GLANCE_GESTURE                 = 24;
    private static final int TYPE_PICK_UP_GESTURE                = 25;
    private static final int TYPE_WRIST_TILT_GESTURE             = 26;
    private static final int TYPE_DEVICE_ORIENTATION             = 27;
    private static final int TYPE_POSE_6DOF                      = 28;
    private static final int TYPE_STATIONARY_DETECT              = 29;
    private static final int TYPE_MOTION_DETECT                  = 30;
    private static final int TYPE_HEART_BEAT                     = 31;
    private static final int TYPE_LOW_LATENCY_OFFBODY_DETECT     = 34;
    private static final int TYPE_ACCELEROMETER_UNCALIBRATED     = 35;
    private static final int TYPE_HINGE_ANGLE                    = 36;
    private static final int TYPE_HEADING                        = 37; // API 33+

    private static String mapSensorType(int type) {
        switch (type) {
            // ── Motion ────────────────────────────────────────────────────────
            case Sensor.TYPE_ACCELEROMETER:              return "accelerometer";
            case Sensor.TYPE_LINEAR_ACCELERATION:        return "linear-acceleration";
            case Sensor.TYPE_GRAVITY:                    return "gravity";
            case TYPE_ACCELEROMETER_UNCALIBRATED:        return "accelerometer-raw";
            case Sensor.TYPE_GYROSCOPE:                  return "gyroscope";
            case TYPE_GYROSCOPE_UNCALIBRATED:            return "gyroscope-raw";
            case TYPE_SIGNIFICANT_MOTION:                return "significant-motion";
            case TYPE_TILT_DETECTOR:                     return "tilt";
            case TYPE_STATIONARY_DETECT:                 return "stationary";
            case TYPE_MOTION_DETECT:                     return "motion";
            case TYPE_WAKE_GESTURE:                      return "wake-gesture";
            case TYPE_GLANCE_GESTURE:                    return "glance-gesture";
            case TYPE_PICK_UP_GESTURE:                   return "pick-up";
            case TYPE_WRIST_TILT_GESTURE:                return "wrist-tilt";
            // ── Orientation ───────────────────────────────────────────────────
            case Sensor.TYPE_ROTATION_VECTOR:            return "orientation";
            case TYPE_GAME_ROTATION_VECTOR:              return "orientation-game";
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:return "orientation-geo";
            case TYPE_DEVICE_ORIENTATION:                return "device-orientation";
            case TYPE_HEADING:                           return "heading";
            case TYPE_POSE_6DOF:                         return "pose-6dof";
            // ── Magnetometer ──────────────────────────────────────────────────
            case Sensor.TYPE_MAGNETIC_FIELD:             return "magnetometer";
            case TYPE_MAGNETIC_FIELD_UNCALIBRATED:       return "magnetometer-raw";
            // ── Environment ───────────────────────────────────────────────────
            case Sensor.TYPE_LIGHT:                      return "light";
            case Sensor.TYPE_PRESSURE:                   return "barometer";
            case TYPE_RELATIVE_HUMIDITY:                 return "humidity";
            case TYPE_AMBIENT_TEMPERATURE:               return "temperature";
            // ── Proximity / body ─────────────────────────────────────────────
            case Sensor.TYPE_PROXIMITY:                  return "proximity";
            case TYPE_HINGE_ANGLE:                       return "hinge-angle";
            case TYPE_LOW_LATENCY_OFFBODY_DETECT:        return "off-body";
            // ── Activity / biometric ─────────────────────────────────────────
            case Sensor.TYPE_STEP_COUNTER:               return "steps";
            case Sensor.TYPE_STEP_DETECTOR:              return "step-detector";
            case Sensor.TYPE_HEART_RATE:                 return "heart-rate";
            case TYPE_HEART_BEAT:                        return "heart-beat";
            default:
                // Report unknown sensor types generically so they're still collected.
                return "sensor-" + type;
        }
    }

    private static String sensorUnit(int type) {
        switch (type) {
            case Sensor.TYPE_ACCELEROMETER:
            case Sensor.TYPE_LINEAR_ACCELERATION:
            case Sensor.TYPE_GRAVITY:
            case TYPE_ACCELEROMETER_UNCALIBRATED:        return "m/s²";
            case Sensor.TYPE_GYROSCOPE:
            case TYPE_GYROSCOPE_UNCALIBRATED:            return "rad/s";
            case Sensor.TYPE_MAGNETIC_FIELD:
            case TYPE_MAGNETIC_FIELD_UNCALIBRATED:       return "µT";
            case Sensor.TYPE_ROTATION_VECTOR:
            case TYPE_GAME_ROTATION_VECTOR:
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:return "quaternion";
            case TYPE_DEVICE_ORIENTATION:                return "rotation";
            case TYPE_POSE_6DOF:                         return "pose";
            case Sensor.TYPE_LIGHT:                      return "lux";
            case Sensor.TYPE_PRESSURE:                   return "hPa";
            case TYPE_RELATIVE_HUMIDITY:                 return "%";
            case TYPE_AMBIENT_TEMPERATURE:               return "°C";
            case Sensor.TYPE_PROXIMITY:                  return "cm";
            case TYPE_HINGE_ANGLE:
            case TYPE_HEADING:                           return "°";
            case Sensor.TYPE_STEP_COUNTER:               return "steps";
            case Sensor.TYPE_STEP_DETECTOR:
            case TYPE_SIGNIFICANT_MOTION:
            case TYPE_TILT_DETECTOR:
            case TYPE_STATIONARY_DETECT:
            case TYPE_MOTION_DETECT:
            case TYPE_WAKE_GESTURE:
            case TYPE_GLANCE_GESTURE:
            case TYPE_PICK_UP_GESTURE:
            case TYPE_WRIST_TILT_GESTURE:
            case TYPE_LOW_LATENCY_OFFBODY_DETECT:        return "event";
            case Sensor.TYPE_HEART_RATE:                 return "bpm";
            case TYPE_HEART_BEAT:                        return "confidence";
            default:                                     return "value";
        }
    }

    private static double[] copyValues(float[] values) {
        if (values == null || values.length == 0) return new double[0];
        double[] out = new double[values.length];
        for (int i = 0; i < values.length; i++) {
            out[i] = values[i];
        }
        return out;
    }

    private static double normalizeValue(SensorSnapshot snapshot) {
        if (snapshot.values == null || snapshot.values.length == 0) return 0d;
        if (snapshot.values.length == 1) return snapshot.values[0];
        double sum = 0d;
        for (double v : snapshot.values) sum += v * v;
        return Math.sqrt(sum);
    }

    /**
     * Deterministic ed25519 keypair for sensor signing.
     *
     * Seed derivation:
     *   SHA-512(postingKey + ":" + deviceId + ":" + board + ":" + hardware
     *           + ":btcpc-sensor-v1:" + sensorId)[0:32]
     *
     * postingKey = account's 32-byte ed25519 seed (64 hex chars, from AppPrefs).
     *   Required to derive any sensor key. An attacker with ADB access to the
     *   device cannot forge signatures without also knowing this secret.
     *
     * deviceId = android_id (stored in SharedPreferences as sensor_device_name).
     *   Binds the key to this physical device. Same android_id survives reinstall
     *   on the same device + Google account; resets on factory reset.
     *
     * sensorId encodes the owner account: "account/deviceId-sensorType"
     *   → new owner on same device registers a different account → different sensorId
     *     → different sensor key, while deviceId in the name shows it's the same hardware.
     *
     * Recovery: same posting key + same device → same sensor keys (survives reinstall).
     * New owner: different account → different postingKey → entirely different keys.
     * Factory reset: new deviceId → new keys → rotate via /register-key.
     *
     * Payload format (matches sensorKeystore.js readingPayload()):
     *   "sensor_id|value|device_timestamp_ms"
     */
    private static final class SensorSigner {
        // SPKI DER prefix for ed25519 (12 bytes): SEQUENCE { SEQUENCE { OID Ed25519 } BIT STRING }
        private static final byte[] SPKI_PREFIX = {
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00
        };

        private final EdDSAPrivateKey privateKey;
        final String publicKeySpkiHex;
        private final String sensorId;

        SensorSigner(String postingKeyHex, String deviceId, String sensorId) throws Exception {
            this.sensorId = sensorId;
            // postingKeyHex is the account's 32-byte ed25519 seed (64 hex chars).
            // Without it an attacker with ADB access cannot derive the sensor key.
            // deviceId (android_id) binds the key to this specific device.
            String material = postingKeyHex + ":" + deviceId + ":" + Build.BOARD + ":"
                    + Build.HARDWARE + ":btcpc-sensor-v1:" + sensorId;
            byte[] hash = MessageDigest.getInstance("SHA-512")
                    .digest(material.getBytes(StandardCharsets.UTF_8));
            byte[] seed = java.util.Arrays.copyOf(hash, 32);

            EdDSAParameterSpec spec = EdDSANamedCurveTable.getByName(EdDSANamedCurveTable.ED_25519);
            EdDSAPrivateKeySpec privSpec = new EdDSAPrivateKeySpec(seed, spec);
            this.privateKey = new EdDSAPrivateKey(privSpec);
            java.util.Arrays.fill(seed, (byte) 0);   // wipe seed from heap

            byte[] rawPub = new EdDSAPublicKey(new EdDSAPublicKeySpec(privSpec.getA(), spec)).getAbyte();
            byte[] spki = new byte[44];
            System.arraycopy(SPKI_PREFIX, 0, spki, 0, 12);
            System.arraycopy(rawPub, 0, spki, 12, 32);
            this.publicKeySpkiHex = toHex(spki);
        }

        String sign(double value, long timestampMs) throws Exception {
            String payload = sensorId + "|" + value + "|" + timestampMs;
            EdDSAEngine eng = new EdDSAEngine(MessageDigest.getInstance("SHA-512"));
            eng.initSign(privateKey);
            eng.update(payload.getBytes(StandardCharsets.UTF_8));
            return toHex(eng.sign());
        }

        private static String toHex(byte[] bytes) {
            StringBuilder sb = new StringBuilder(bytes.length * 2);
            for (byte b : bytes) sb.append(String.format("%02x", b & 0xff));
            return sb.toString();
        }
    }

    private static final class SensorSnapshot {
        String sensorId;
        String type;
        String unit;
        long timestamp;
        double[] values;
        int androidType;
        String name;
        String vendor;

        org.json.JSONArray valuesToJsonArray() {
            org.json.JSONArray array = new org.json.JSONArray();
            if (values != null) {
                for (double v : values) {
                    try {
                        array.put(v);
                    } catch (Exception ignored) {
                        // ignore malformed numeric entries
                    }
                }
            }
            return array;
        }
    }
}
