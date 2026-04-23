package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
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

import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;

public class NativeSensorService extends Service implements SensorEventListener, LocationListener {

    private static final String TAG = "BTCPCSensor";
    private static final String CHANNEL_ID = "btcpc_sensors";
    private static final int NOTIFICATION_ID = 9440;
    private static final String PREFS = "btcpc_native_state";
    private static final String API_BASE = "https://btcpc.net/api";
    private static final MediaType JSON = MediaType.get("application/json; charset=utf-8");

    private final OkHttpClient client = new OkHttpClient.Builder().retryOnConnectionFailure(true).build();
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();
    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private final Map<String, SensorSnapshot> snapshots = Collections.synchronizedMap(new HashMap<>());
    private final Set<String> registeredSensors = Collections.synchronizedSet(new HashSet<>());
    private volatile boolean running;
    private volatile String account;
    private volatile String deviceName;
    private volatile String jwt;
    private SensorManager sensorManager;
    private LocationManager locationManager;
    private ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;
    private BroadcastReceiver batteryReceiver;
    private volatile boolean gpsUpdatesActive;

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
        scheduler.scheduleAtFixedRate(this::flushSnapshotsSafe, 10, 30, TimeUnit.SECONDS);
        persistState("Sensors: running");
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        running = false;
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
            case Sensor.TYPE_ACCELEROMETER:
            case Sensor.TYPE_LINEAR_ACCELERATION:
            case Sensor.TYPE_GRAVITY:
            case 16: // GYROSCOPE_UNCALIBRATED
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MOTION)
                    && mapSensorType(type) != null;
            case Sensor.TYPE_GYROSCOPE:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MOTION);
            case Sensor.TYPE_ROTATION_VECTOR:
            case Sensor.TYPE_GAME_ROTATION_VECTOR:
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ORIENTATION);
            case Sensor.TYPE_LIGHT:
            case Sensor.TYPE_PRESSURE:
            case 13: // AMBIENT_TEMPERATURE
            case 12: // RELATIVE_HUMIDITY
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ENVIRONMENT);
            case Sensor.TYPE_MAGNETIC_FIELD:
            case 14: // MAGNETIC_FIELD_UNCALIBRATED
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MAGNETOMETER);
            case Sensor.TYPE_PROXIMITY:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_PROXIMITY);
            case Sensor.TYPE_STEP_COUNTER:
            case Sensor.TYPE_STEP_DETECTOR:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_STEPS);
            case Sensor.TYPE_HEART_RATE:
                return prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_HEARTRATE);
            default:
                return mapSensorType(type) != null;
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
                    int level  = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
                    int scale  = intent.getIntExtra(BatteryManager.EXTRA_SCALE, 100);
                    int status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1);
                    if (level < 0 || scale <= 0) return;
                    double pct = (level * 100.0) / scale;
                    boolean charging = status == BatteryManager.BATTERY_STATUS_CHARGING
                            || status == BatteryManager.BATTERY_STATUS_FULL;

                    SensorSnapshot snap = new SensorSnapshot();
                    snap.type = "battery";
                    snap.unit = "%";
                    snap.sensorId = account + "/" + deviceName + "-battery";
                    snap.timestamp = System.currentTimeMillis();
                    snap.values = new double[]{pct};
                    snap.androidType = -1;
                    snap.name = "Battery";
                    snap.vendor = Build.MANUFACTURER;
                    // Stash charging flag in extra field via metadata later
                    snapshots.put("battery", snap);
                } catch (Exception e) {
                    Log.w(TAG, "Battery receiver error: " + e.getMessage());
                }
            }
        };
        IntentFilter filter = new IntentFilter(Intent.ACTION_BATTERY_CHANGED);
        registerReceiver(batteryReceiver, filter);
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
        snap.values = new double[]{location.getLatitude(), location.getLongitude(),
                location.getAltitude(), location.getAccuracy()};
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
                submitReading(snapshot);
            }
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
            postJson(API_BASE + "/sensors", body);
            registeredSensors.add(sensorId);
            persistState("Sensors: registered " + sensorId);
        } catch (Exception e) {
            Log.w(TAG, "register failed: " + e.getMessage());
        }
    }

    private void submitReading(SensorSnapshot snapshot) {
        try {
            double value = normalizeValue(snapshot);
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

            String url = API_BASE + "/sensors/" + encode(snapshot.sensorId) + "/readings";
            postJson(url, body);
            persistState(String.format(Locale.US, "Sensors: submitted %s=%.3f", snapshot.type, value));
        } catch (Exception e) {
            Log.w(TAG, "submit failed: " + e.getMessage());
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
    private static final int TYPE_MAGNETIC_FIELD_UNCALIBRATED = 14;
    private static final int TYPE_GAME_ROTATION_VECTOR        = 15;
    private static final int TYPE_GYROSCOPE_UNCALIBRATED      = 16;
    private static final int TYPE_RELATIVE_HUMIDITY           = 12;
    private static final int TYPE_AMBIENT_TEMPERATURE         = 13;
    private static final int TYPE_STATIONARY_DETECT           = 29;
    private static final int TYPE_MOTION_DETECT               = 30;
    private static final int TYPE_HINGE_ANGLE                 = 36;
    private static final int TYPE_HEADING                     = 35;

    private static String mapSensorType(int type) {
        switch (type) {
            case Sensor.TYPE_ACCELEROMETER:              return "accelerometer";
            case Sensor.TYPE_LINEAR_ACCELERATION:        return "linear-acceleration";
            case Sensor.TYPE_GRAVITY:                    return "gravity";
            case Sensor.TYPE_GYROSCOPE:                  return "gyroscope";
            case TYPE_GYROSCOPE_UNCALIBRATED:            return "gyroscope-raw";
            case Sensor.TYPE_ROTATION_VECTOR:            return "orientation";
            case TYPE_GAME_ROTATION_VECTOR:              return "orientation-game";
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:return "orientation-geo";
            case Sensor.TYPE_LIGHT:                      return "light";
            case Sensor.TYPE_MAGNETIC_FIELD:             return "magnetometer";
            case TYPE_MAGNETIC_FIELD_UNCALIBRATED:       return "magnetometer-raw";
            case Sensor.TYPE_PRESSURE:                   return "barometer";
            case Sensor.TYPE_PROXIMITY:                  return "proximity";
            case Sensor.TYPE_STEP_COUNTER:               return "steps";
            case Sensor.TYPE_STEP_DETECTOR:              return "step-detector";
            case Sensor.TYPE_HEART_RATE:                 return "heart-rate";
            case TYPE_RELATIVE_HUMIDITY:                 return "humidity";
            case TYPE_AMBIENT_TEMPERATURE:               return "temperature";
            case TYPE_HINGE_ANGLE:                       return "hinge-angle";
            case TYPE_HEADING:                           return "heading";
            case TYPE_STATIONARY_DETECT:                 return "stationary";
            case TYPE_MOTION_DETECT:                     return "motion";
            default:                                     return null;
        }
    }

    private static String sensorUnit(int type) {
        switch (type) {
            case Sensor.TYPE_ACCELEROMETER:
            case Sensor.TYPE_LINEAR_ACCELERATION:
            case Sensor.TYPE_GRAVITY:                    return "m/s²";
            case Sensor.TYPE_GYROSCOPE:
            case TYPE_GYROSCOPE_UNCALIBRATED:            return "rad/s";
            case Sensor.TYPE_MAGNETIC_FIELD:
            case TYPE_MAGNETIC_FIELD_UNCALIBRATED:       return "µT";
            case Sensor.TYPE_ROTATION_VECTOR:
            case TYPE_GAME_ROTATION_VECTOR:
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:return "quaternion";
            case Sensor.TYPE_LIGHT:                      return "lux";
            case Sensor.TYPE_PRESSURE:                   return "hPa";
            case Sensor.TYPE_PROXIMITY:                  return "cm";
            case Sensor.TYPE_STEP_COUNTER:               return "steps";
            case Sensor.TYPE_STEP_DETECTOR:              return "event";
            case Sensor.TYPE_HEART_RATE:                 return "bpm";
            case TYPE_RELATIVE_HUMIDITY:                 return "%";
            case TYPE_AMBIENT_TEMPERATURE:               return "°C";
            case TYPE_HINGE_ANGLE:                       return "°";
            case TYPE_HEADING:                           return "°";
            case TYPE_STATIONARY_DETECT:
            case TYPE_MOTION_DETECT:                     return "event";
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
