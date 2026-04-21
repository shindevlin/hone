package network.btcpc.app;

import android.os.Handler;
import android.os.Looper;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.IOException;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;
import okhttp3.ResponseBody;

/**
 * ChainApi — lightweight async API client for btcpc.net.
 *
 * All methods fire requests on OkHttp's dispatcher threads and deliver
 * results back on the main thread via the provided callback.
 */
public class ChainApi {

    // ---------- callback interfaces ----------

    public interface BalanceCallback {
        /** Called on the main thread with the parsed balance response. */
        void onSuccess(String username, String address, double btcpcBalance, double delegatedBalance);
        /** Called on the main thread when the request fails. */
        void onError(String message);
    }

    public interface HistoryCallback {
        /** Called on the main thread with the raw JSONArray of history items. */
        void onSuccess(JSONArray history);
        /** Called on the main thread when the request fails. */
        void onError(String message);
    }

    public interface PhoneModelsCallback {
        /** model ids (e.g. ["qwen2.5-0.5b", "llama-3.2-1b"]) and display names */
        void onSuccess(java.util.List<String> ids, java.util.List<String> names);
        void onError(String message);
    }

    public interface LoginCallback {
        void onSuccess(String account, String token);
        void onError(String message);
    }

    public interface RelayStatusCallback {
        /** Called on the main thread with parsed relay status fields. */
        void onSuccess(boolean ok, boolean usbConnected, boolean bleConnected, String lastReading);
        /** Called on the main thread when the request fails (relay service not running, etc.). */
        void onError(String message);
    }

    // ---------- internals ----------

    private static final OkHttpClient CLIENT = new OkHttpClient.Builder()
            .connectTimeout(10, java.util.concurrent.TimeUnit.SECONDS)
            .readTimeout(15, java.util.concurrent.TimeUnit.SECONDS)
            .build();

    private static final Handler MAIN = new Handler(Looper.getMainLooper());

    private ChainApi() {}

    // ---------- public methods ----------

    private static final MediaType JSON_MEDIA = MediaType.get("application/json; charset=utf-8");

    /**
     * Fetch models suitable for on-device phone mining from the chain registry.
     * GET /api/models/registry — filters to models with size_mb <= threshold.
     */
    public static void fetchPhoneModels(String apiBase, PhoneModelsCallback callback) {
        String url = apiBase + "/api/models/registry";
        Request request = new Request.Builder().url(url).get().build();

        CLIENT.newCall(request).enqueue(new Callback() {
            @Override
            public void onFailure(Call call, IOException e) {
                MAIN.post(() -> callback.onError("Network error: " + e.getMessage()));
            }

            @Override
            public void onResponse(Call call, Response response) throws IOException {
                try (ResponseBody rb = response.body()) {
                    if (!response.isSuccessful() || rb == null) {
                        MAIN.post(() -> callback.onError("Server error " + response.code()));
                        return;
                    }
                    JSONObject json = new JSONObject(rb.string());
                    java.util.List<String> ids   = new java.util.ArrayList<>();
                    java.util.List<String> names = new java.util.ArrayList<>();

                    // Registry is a map of id → metadata
                    java.util.Iterator<String> keys = json.keys();
                    while (keys.hasNext()) {
                        String id = keys.next();
                        JSONObject meta = json.optJSONObject(id);
                        if (meta == null) continue;
                        // Only include small models (≤ 4 GB) — skip deprecated
                        if ("deprecated".equals(meta.optString("status"))) continue;
                        long sizeMb = meta.optLong("size_mb", 9999);
                        if (sizeMb > 4096) continue;
                        String name = meta.optString("name", id);
                        ids.add(id);
                        names.add(name + (sizeMb > 0 ? " (" + sizeMb + " MB)" : ""));
                    }

                    if (ids.isEmpty()) {
                        // Fallback list if registry is empty or all filtered
                        ids.add("qwen2.5-0.5b");   names.add("Qwen2.5-0.5B (~400 MB)");
                        ids.add("qwen3-0.6b");      names.add("Qwen3-0.6B (~450 MB)");
                        ids.add("smollm2-360m");    names.add("SmolLM2-360M (~280 MB)");
                        ids.add("llama-3.2-1b");    names.add("Llama 3.2 1B (~700 MB)");
                        ids.add("deepseek-r1-1.5b");names.add("DeepSeek-R1 1.5B (~1.1 GB)");
                        ids.add("qwen3-1.7b");      names.add("Qwen3-1.7B (~1.3 GB)");
                        ids.add("gemma-3-1b");      names.add("Gemma-3 1B (~850 MB)");
                        ids.add("phi-3.5-mini");    names.add("Phi-3.5 Mini (~2.2 GB)");
                        ids.add("llama-3.2-3b");    names.add("Llama 3.2 3B (~2.0 GB)");
                        ids.add("qwen3-4b");        names.add("Qwen3-4B (~2.6 GB)");
                    }
                    MAIN.post(() -> callback.onSuccess(ids, names));
                } catch (Exception e) {
                    MAIN.post(() -> callback.onError("Parse error: " + e.getMessage()));
                }
            }
        });
    }

    /**
     * Authenticate with the chain node.
     * POST /api/user/login { username, password } → { token, user.username }
     */
    public static void login(String username, String password, String apiBase,
                             LoginCallback callback) {
        String url = apiBase + "/api/user/login";
        String body;
        try {
            JSONObject obj = new JSONObject();
            obj.put("username", username);
            obj.put("password", password);
            body = obj.toString();
        } catch (Exception e) {
            MAIN.post(() -> callback.onError("Build error: " + e.getMessage()));
            return;
        }
        Request request = new Request.Builder()
                .url(url)
                .post(RequestBody.create(body, JSON_MEDIA))
                .build();

        CLIENT.newCall(request).enqueue(new Callback() {
            @Override
            public void onFailure(Call call, IOException e) {
                MAIN.post(() -> callback.onError("Network error: " + e.getMessage()));
            }

            @Override
            public void onResponse(Call call, Response response) throws IOException {
                try (ResponseBody rb = response.body()) {
                    String raw = rb != null ? rb.string() : "";
                    JSONObject json = new JSONObject(raw);
                    if (!response.isSuccessful()) {
                        String msg = json.optString("error", "Login failed (" + response.code() + ")");
                        MAIN.post(() -> callback.onError(msg));
                        return;
                    }
                    String token   = json.optString("token", "");
                    JSONObject usr = json.optJSONObject("user");
                    String acct    = usr != null ? usr.optString("username", username) : username;
                    if (token.isEmpty()) {
                        MAIN.post(() -> callback.onError("No token returned"));
                        return;
                    }
                    MAIN.post(() -> callback.onSuccess(acct, token));
                } catch (Exception e) {
                    MAIN.post(() -> callback.onError("Parse error: " + e.getMessage()));
                }
            }
        });
    }

    /**
     * Fetch wallet balance for the authenticated account.
     *
     * @param account  BTCPC account name (used to build display, not sent to API — the JWT identifies the user)
     * @param jwt      Bearer token
     * @param apiBase  e.g. "https://btcpc.net"
     * @param callback result on main thread
     */
    public static void fetchBalance(String account, String jwt, String apiBase,
                                    BalanceCallback callback) {
        String url = apiBase + "/api/wallet/balance";
        Request request = new Request.Builder()
                .url(url)
                .addHeader("Authorization", "Bearer " + jwt)
                .get()
                .build();

        CLIENT.newCall(request).enqueue(new Callback() {
            @Override
            public void onFailure(Call call, IOException e) {
                MAIN.post(() -> callback.onError("Network error: " + e.getMessage()));
            }

            @Override
            public void onResponse(Call call, Response response) throws IOException {
                try (ResponseBody body = response.body()) {
                    if (!response.isSuccessful() || body == null) {
                        final int code = response.code();
                        MAIN.post(() -> callback.onError("Server error " + code));
                        return;
                    }
                    String raw = body.string();
                    JSONObject json = new JSONObject(raw);
                    if (!json.optBoolean("success", false)) {
                        String msg = json.optString("error", "Unknown error");
                        MAIN.post(() -> callback.onError(msg));
                        return;
                    }
                    String username = json.optString("username", account);
                    String address  = json.optString("address", "");
                    JSONObject balObj = json.optJSONObject("balance");
                    double btcpc = balObj != null ? balObj.optDouble("BTCPC", 0) : 0;
                    double delegated = json.optDouble("delegatedBalance", 0);
                    MAIN.post(() -> callback.onSuccess(username, address, btcpc, delegated));
                } catch (Exception e) {
                    MAIN.post(() -> callback.onError("Parse error: " + e.getMessage()));
                }
            }
        });
    }

    /**
     * Fetch on-chain history for an account (public endpoint — no JWT required).
     *
     * @param account  BTCPC account name
     * @param apiBase  e.g. "https://btcpc.net"
     * @param callback result on main thread
     */
    public static void fetchHistory(String account, String apiBase, HistoryCallback callback) {
        if (account == null || account.trim().isEmpty()) {
            MAIN.post(() -> callback.onError("No account configured"));
            return;
        }
        String url = apiBase + "/api/explorer/account/"
                + account.trim() + "/history?limit=50";
        Request request = new Request.Builder()
                .url(url)
                .get()
                .build();

        CLIENT.newCall(request).enqueue(new Callback() {
            @Override
            public void onFailure(Call call, IOException e) {
                MAIN.post(() -> callback.onError("Network error: " + e.getMessage()));
            }

            @Override
            public void onResponse(Call call, Response response) throws IOException {
                try (ResponseBody body = response.body()) {
                    if (!response.isSuccessful() || body == null) {
                        final int code = response.code();
                        MAIN.post(() -> callback.onError("Server error " + code));
                        return;
                    }
                    String raw = body.string();
                    JSONObject json = new JSONObject(raw);
                    JSONArray history = json.optJSONArray("history");
                    if (history == null) history = new JSONArray();
                    final JSONArray result = history;
                    MAIN.post(() -> callback.onSuccess(result));
                } catch (Exception e) {
                    MAIN.post(() -> callback.onError("Parse error: " + e.getMessage()));
                }
            }
        });
    }

    /**
     * Poll the local relay status endpoint exposed by LocalRelayService on port 6942.
     *
     * @param callback result on main thread
     */
    public static void fetchRelayStatus(RelayStatusCallback callback) {
        Request request = new Request.Builder()
                .url("http://localhost:6942/_relay/status")
                .get()
                .build();

        CLIENT.newCall(request).enqueue(new Callback() {
            @Override
            public void onFailure(Call call, IOException e) {
                MAIN.post(() -> callback.onError("Relay not reachable: " + e.getMessage()));
            }

            @Override
            public void onResponse(Call call, Response response) throws IOException {
                try (ResponseBody body = response.body()) {
                    if (!response.isSuccessful() || body == null) {
                        MAIN.post(() -> callback.onError("Relay error " + response.code()));
                        return;
                    }
                    String raw = body.string();
                    JSONObject json = new JSONObject(raw);
                    boolean ok  = json.optBoolean("ok", false);
                    boolean usb = json.optBoolean("usb_connected", false);
                    boolean ble = json.optBoolean("ble_connected", false);
                    String last = json.isNull("last_reading")
                            ? null
                            : json.optString("last_reading", null);
                    MAIN.post(() -> callback.onSuccess(ok, usb, ble, last));
                } catch (Exception e) {
                    MAIN.post(() -> callback.onError("Parse error: " + e.getMessage()));
                }
            }
        });
    }
}
