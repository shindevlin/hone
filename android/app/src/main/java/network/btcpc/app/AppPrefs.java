package network.btcpc.app;

import android.content.Context;
import android.content.SharedPreferences;

/**
 * AppPrefs — thin wrapper around the SharedPreferences store used by all
 * BTCPC native components.  The same file name ("btcpc_native_state") is
 * used by the background services so every component reads from a single
 * source of truth.
 */
public class AppPrefs {

    public static final String PREFS_NAME = "btcpc_native_state";

    // ---- user-facing settings keys ----
    public static final String KEY_ACCOUNT     = "account";
    public static final String KEY_JWT         = "jwt";
    public static final String KEY_API_URL     = "apiUrl";
    public static final String KEY_RELAY_URL   = "relayUrl";   // WebSocket relay for clock peer
    public static final String KEY_DEVICE_NAME = "sensor_device_name";

    // ---- posting key (secp256k1 private key for signing work proofs) ----
    public static final String KEY_POSTING_KEY = "postingKey";

    // ---- miner / storage config ----
    public static final String KEY_MINER_MODEL      = "minerModel";
    public static final String KEY_STORAGE_QUOTA_MB = "storageQuotaMb";
    public static final String DEFAULT_MINER_MODEL  = "qwen2.5-0.5b";
    public static final int    DEFAULT_QUOTA_MB     = 1024;

    // ---- per-sensor toggles ----
    public static final String KEY_SENSOR_MOTION      = "sensorMotion";
    public static final String KEY_SENSOR_ORIENTATION = "sensorOrientation";
    public static final String KEY_SENSOR_ENVIRONMENT = "sensorEnvironment";
    public static final String KEY_SENSOR_MAGNETOMETER= "sensorMagnetometer";
    public static final String KEY_SENSOR_PROXIMITY   = "sensorProximity";
    public static final String KEY_SENSOR_STEPS       = "sensorSteps";
    public static final String KEY_SENSOR_HEARTRATE   = "sensorHeartrate";
    public static final String KEY_SENSOR_GPS         = "sensorGps";
    public static final String KEY_SENSOR_BATTERY     = "sensorBattery";

    // ---- service-enabled toggles ----
    public static final String KEY_MINER_ENABLED   = "minerEnabled";
    public static final String KEY_CLOCK_ENABLED   = "clockEnabled";
    public static final String KEY_SENSORS_ENABLED = "sensorsEnabled";
    public static final String KEY_STORAGE_ENABLED = "storageEnabled";

    // ---- Flipper Zero pairing (BLE MAC-based) ----
    public static final String KEY_FLIPPER_BLE_MAC  = "flipperBleMac";
    public static final String KEY_FLIPPER_BLE_NAME = "flipperBleName";

    // ---- service status (written by background services) ----
    public static final String KEY_RELAY_STATE   = "relay_state";
    public static final String KEY_CLOCK_STATE   = "clock_state";
    public static final String KEY_SENSOR_STATE  = "sensor_state";
    public static final String KEY_MINER_STATE   = "miner_state";
    public static final String KEY_STORAGE_STATE = "storage_state";
    public static final String KEY_LAST_EVENT    = "last_event";

    // ---- defaults ----
    public static final String DEFAULT_API_URL     = "https://btcpc.net";
    public static final String DEFAULT_RELAY_URL   = "wss://btcpc-relay.shindevlin.workers.dev/ws";
    public static final String DEFAULT_ACCOUNT     = "";
    public static final String DEFAULT_DEVICE_NAME = "android-phone";

    private final SharedPreferences prefs;

    public AppPrefs(Context context) {
        prefs = context.getApplicationContext()
                       .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE);
    }

    // ---- getters ----

    public String getAccount() {
        return prefs.getString(KEY_ACCOUNT, DEFAULT_ACCOUNT);
    }

    public String getJwt() {
        return prefs.getString(KEY_JWT, "");
    }

    public String getPostingKey() {
        return prefs.getString(KEY_POSTING_KEY, "");
    }

    public void setPostingKey(String value) {
        prefs.edit().putString(KEY_POSTING_KEY, value.trim()).apply();
    }

    public String getApiUrl() {
        String url = prefs.getString(KEY_API_URL, DEFAULT_API_URL);
        if (url == null || url.trim().isEmpty()) url = DEFAULT_API_URL;
        // strip trailing slash
        while (url.endsWith("/")) url = url.substring(0, url.length() - 1);
        return url;
    }

    public String getRelayUrl() {
        String url = prefs.getString(KEY_RELAY_URL, DEFAULT_RELAY_URL);
        return (url == null || url.trim().isEmpty()) ? DEFAULT_RELAY_URL : url.trim();
    }

    public void setRelayUrl(String value) {
        prefs.edit().putString(KEY_RELAY_URL, value.trim()).apply();
    }

    public String getDeviceName() {
        String d = prefs.getString(KEY_DEVICE_NAME, DEFAULT_DEVICE_NAME);
        return (d == null || d.trim().isEmpty()) ? DEFAULT_DEVICE_NAME : d;
    }

    public boolean isSensorEnabled(String key) {
        boolean def = !KEY_SENSOR_HEARTRATE.equals(key) && !KEY_SENSOR_GPS.equals(key);
        return prefs.getBoolean(key, def);
    }
    public void setSensorEnabled(String key, boolean v) {
        prefs.edit().putBoolean(key, v).apply();
    }

    public boolean isMinerEnabled() {
        return prefs.getBoolean(KEY_MINER_ENABLED, false);
    }

    public boolean isClockEnabled() {
        return prefs.getBoolean(KEY_CLOCK_ENABLED, true);
    }

    public boolean isSensorsEnabled() {
        return prefs.getBoolean(KEY_SENSORS_ENABLED, true);
    }

    public boolean isStorageEnabled() {
        return prefs.getBoolean(KEY_STORAGE_ENABLED, false);
    }

    public String getRelayState() {
        return prefs.getString(KEY_RELAY_STATE, "stopped");
    }

    public String getClockState() {
        return prefs.getString(KEY_CLOCK_STATE, "stopped");
    }

    public String getSensorState() {
        return prefs.getString(KEY_SENSOR_STATE, "stopped");
    }

    public String getMinerModel() {
        String m = prefs.getString(KEY_MINER_MODEL, DEFAULT_MINER_MODEL);
        return (m == null || m.trim().isEmpty()) ? DEFAULT_MINER_MODEL : m.trim();
    }

    public void setMinerModel(String value) {
        prefs.edit().putString(KEY_MINER_MODEL, value.trim()).apply();
    }

    public int getStorageQuotaMb() {
        return prefs.getInt(KEY_STORAGE_QUOTA_MB, DEFAULT_QUOTA_MB);
    }

    public void setStorageQuotaMb(int mb) {
        prefs.edit().putInt(KEY_STORAGE_QUOTA_MB, mb).apply();
    }

    public String getMinerState() {
        return prefs.getString(KEY_MINER_STATE, "");
    }

    public String getStorageState() {
        return prefs.getString(KEY_STORAGE_STATE, "");
    }

    public void setMinerState(String value) {
        prefs.edit().putString(KEY_MINER_STATE, value).apply();
    }

    public void setStorageState(String value) {
        prefs.edit().putString(KEY_STORAGE_STATE, value).apply();
    }

    public String getLastEvent() {
        return prefs.getString(KEY_LAST_EVENT, "");
    }

    // ---- setters ----

    public void setAccount(String value) {
        prefs.edit().putString(KEY_ACCOUNT, value.trim()).apply();
    }

    public void setJwt(String value) {
        prefs.edit().putString(KEY_JWT, value.trim()).apply();
    }

    public void setApiUrl(String value) {
        prefs.edit().putString(KEY_API_URL, value.trim()).apply();
    }

    public void setDeviceName(String value) {
        prefs.edit().putString(KEY_DEVICE_NAME, value.trim()).apply();
    }

    public void setMinerEnabled(boolean enabled) {
        prefs.edit().putBoolean(KEY_MINER_ENABLED, enabled).apply();
    }

    public void setClockEnabled(boolean enabled) {
        prefs.edit().putBoolean(KEY_CLOCK_ENABLED, enabled).apply();
    }

    public void setSensorsEnabled(boolean enabled) {
        prefs.edit().putBoolean(KEY_SENSORS_ENABLED, enabled).apply();
    }

    public void setStorageEnabled(boolean enabled) {
        prefs.edit().putBoolean(KEY_STORAGE_ENABLED, enabled).apply();
    }

    // ---- Flipper pairing ----

    public String getFlipperBleMac() {
        return prefs.getString(KEY_FLIPPER_BLE_MAC, "");
    }

    public String getFlipperBleName() {
        return prefs.getString(KEY_FLIPPER_BLE_NAME, "");
    }

    public void setFlipperBle(String mac, String name) {
        prefs.edit()
             .putString(KEY_FLIPPER_BLE_MAC, mac.trim())
             .putString(KEY_FLIPPER_BLE_NAME, name != null ? name.trim() : "")
             .apply();
    }

    public void clearFlipperBle() {
        prefs.edit().remove(KEY_FLIPPER_BLE_MAC).remove(KEY_FLIPPER_BLE_NAME).apply();
    }

    /** Bulk-save all user-facing settings in one edit. */
    public void saveAll(String account, String jwt, String postingKey, String apiUrl, String relayUrl, String deviceName) {
        prefs.edit()
             .putString(KEY_ACCOUNT, account.trim())
             .putString(KEY_JWT, jwt.trim())
             .putString(KEY_POSTING_KEY, postingKey.trim())
             .putString(KEY_API_URL, apiUrl.trim())
             .putString(KEY_RELAY_URL, relayUrl.trim())
             .putString(KEY_DEVICE_NAME, deviceName.trim())
             .apply();
    }

    /** Raw access for callers that need to read arbitrary keys written by services. */
    public SharedPreferences raw() {
        return prefs;
    }
}
