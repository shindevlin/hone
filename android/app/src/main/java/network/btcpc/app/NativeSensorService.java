package network.btcpc.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
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

public class NativeSensorService extends Service implements SensorEventListener {

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
    private SensorManager sensorManager;

    @Override
    public void onCreate() {
        super.onCreate();
        sensorManager = (SensorManager) getSystemService(Context.SENSOR_SERVICE);
        createNotificationChannel();
        loadSettings();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        loadSettings();
        running = true;

        Notification notification = buildNotification("Phone sensors: collecting");
        startForeground(NOTIFICATION_ID, notification);
        registerSensors();
        scheduler.scheduleAtFixedRate(this::flushSnapshotsSafe, 10, 30, TimeUnit.SECONDS);
        persistState("Sensors: running");
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        running = false;
        scheduler.shutdownNow();
        io.shutdownNow();
        if (sensorManager != null) {
            sensorManager.unregisterListener(this);
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
        persistState("Sensors: " + sensorType + " active");
    }

    @Override
    public void onAccuracyChanged(Sensor sensor, int accuracy) {
        // no-op
    }

    private void registerSensors() {
        if (sensorManager == null) return;
        sensorManager.unregisterListener(this);
        registeredSensors.clear();
        List<Sensor> all = sensorManager.getSensorList(Sensor.TYPE_ALL);
        for (Sensor sensor : all) {
            if (mapSensorType(sensor.getType()) == null) continue;
            sensorManager.registerListener(this, sensor, SensorManager.SENSOR_DELAY_NORMAL);
        }
        persistState("Sensors: listeners registered");
    }

    private void flushSnapshotsSafe() {
        if (!running) return;
        io.submit(() -> {
            synchronized (snapshots) {
                for (SensorSnapshot snapshot : snapshots.values()) {
                    if (snapshot == null || snapshot.values == null || snapshot.values.length == 0) continue;
                    ensureRegistered(snapshot);
                    submitReading(snapshot);
                }
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
        Request request = new Request.Builder()
                .url(url)
                .post(RequestBody.create(body.toString(), JSON))
                .build();
        try (Response response = client.newCall(request).execute()) {
            if (!response.isSuccessful()) {
                throw new IllegalStateException("HTTP " + response.code());
            }
        }
    }

    private void loadSettings() {
        SharedPreferences prefs = getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        account = prefs.getString("account", "btcpc-phone");
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

    private static String mapSensorType(int type) {
        switch (type) {
            case Sensor.TYPE_ACCELEROMETER:
                return "accelerometer";
            case Sensor.TYPE_LINEAR_ACCELERATION:
                return "linear-acceleration";
            case Sensor.TYPE_GRAVITY:
                return "gravity";
            case Sensor.TYPE_GYROSCOPE:
                return "gyroscope";
            case Sensor.TYPE_ROTATION_VECTOR:
            case Sensor.TYPE_GAME_ROTATION_VECTOR:
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:
                return "orientation";
            case Sensor.TYPE_LIGHT:
                return "light";
            case Sensor.TYPE_MAGNETIC_FIELD:
                return "magnetometer";
            case Sensor.TYPE_PRESSURE:
                return "barometer";
            case Sensor.TYPE_PROXIMITY:
                return "proximity";
            case Sensor.TYPE_STEP_COUNTER:
            case Sensor.TYPE_STEP_DETECTOR:
                return "steps";
            case Sensor.TYPE_HEART_RATE:
                return "heart-rate";
            default:
                return null;
        }
    }

    private static String sensorUnit(int type) {
        switch (type) {
            case Sensor.TYPE_ACCELEROMETER:
            case Sensor.TYPE_LINEAR_ACCELERATION:
            case Sensor.TYPE_GRAVITY:
            case Sensor.TYPE_GYROSCOPE:
            case Sensor.TYPE_MAGNETIC_FIELD:
            case Sensor.TYPE_ROTATION_VECTOR:
            case Sensor.TYPE_GAME_ROTATION_VECTOR:
            case Sensor.TYPE_GEOMAGNETIC_ROTATION_VECTOR:
                return "magnitude";
            case Sensor.TYPE_LIGHT:
                return "lux";
            case Sensor.TYPE_PRESSURE:
                return "hPa";
            case Sensor.TYPE_PROXIMITY:
                return "cm";
            case Sensor.TYPE_STEP_COUNTER:
            case Sensor.TYPE_STEP_DETECTOR:
                return "steps";
            case Sensor.TYPE_HEART_RATE:
                return "bpm";
            default:
                return "value";
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
