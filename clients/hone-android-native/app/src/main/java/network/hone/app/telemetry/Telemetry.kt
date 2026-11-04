package network.hone.app.telemetry

import android.content.Context
import android.util.Log
import network.hone.app.data.ChainHttp
import network.hone.app.sightings.RoleKeys
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.atomic.AtomicLong

/**
 * Counters and last-event times for the signed device status (DeviceStatus), plus the
 * signing identity every public upload uses. Nothing here holds audio, keys or other
 * people's identifiers: only counts, ages and short error strings.
 */
object Telemetry {
    private const val TAG = "HoneTelemetry"
    const val API = "https://honemesh.net/api"

    // ── counters (process lifetime; persistent ones live in prefs) ─────────────────────
    val wifiScans = AtomicLong(); @Volatile var wifiLastMs = 0L
    @Volatile var bleLastMs = 0L
    val flipperFrames = AtomicLong(); @Volatile var flipperLastMs = 0L
    val birdDetections = AtomicLong(); @Volatile var birdLastMs = 0L
    val sensorAccepted = AtomicLong(); val sensorRejected = AtomicLong()
    @Volatile var sensorLastError = ""
    @Volatile var serviceStartedMs = 0L

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_telemetry", Context.MODE_PRIVATE)

    fun wifiScan() { wifiScans.incrementAndGet(); wifiLastMs = System.currentTimeMillis() }
    fun bleAdvert() { bleLastMs = System.currentTimeMillis() }
    fun flipperFrame() { flipperFrames.incrementAndGet(); flipperLastMs = System.currentTimeMillis() }
    fun birdDetection() { birdDetections.incrementAndGet(); birdLastMs = System.currentTimeMillis() }

    /** Upload result per channel ("birds", "photos", "status", "flipper"): ok/failed counts and last error. */
    fun upload(ctx: Context, channel: String, ok: Boolean, error: String? = null) {
        val p = prefs(ctx); val e = p.edit()
        if (ok) e.putLong("${channel}_ok", p.getLong("${channel}_ok", 0) + 1).putLong("${channel}_ok_ms", System.currentTimeMillis())
        else e.putLong("${channel}_fail", p.getLong("${channel}_fail", 0) + 1)
            .putString("${channel}_err", (error ?: "error").take(160)).putLong("${channel}_err_ms", System.currentTimeMillis())
        e.apply()
    }

    /** Why a channel did not upload this round (no account, no key, no location...). */
    fun skipped(ctx: Context, channel: String, reason: String) {
        prefs(ctx).edit().putString("${channel}_skip", reason.take(80)).putLong("${channel}_skip_ms", System.currentTimeMillis()).apply()
    }

    fun uploadStats(ctx: Context, channel: String, nowMs: Long): Map<String, Any?> {
        val p = prefs(ctx)
        fun age(k: String) = p.getLong(k, 0).takeIf { it > 0 }?.let { (nowMs - it) / 1000 }
        return mapOf(
            "ok" to p.getLong("${channel}_ok", 0), "failed" to p.getLong("${channel}_fail", 0),
            "last_ok_s" to age("${channel}_ok_ms"), "last_error" to p.getString("${channel}_err", null),
            "last_error_s" to age("${channel}_err_ms"), "skip" to p.getString("${channel}_skip", null),
            "skip_s" to age("${channel}_skip_ms"),
        )
    }

    // ── crashes ─────────────────────────────────────────────────────────────────────
    @Volatile private var crashInstalled = false

    fun installCrashHandler(ctx: Context) {
        if (crashInstalled) return
        crashInstalled = true
        val app = ctx.applicationContext
        val prev = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { t, e ->
            runCatching {
                // Class + message + top app frame only (no user data in exception messages here).
                val frame = e.stackTrace.firstOrNull { it.className.startsWith("network.hone") }
                prefs(app).edit().putString("crash", "${e.javaClass.simpleName}: ${e.message?.take(120)} @ ${frame?.fileName}:${frame?.lineNumber} [${t.name}]")
                    .putLong("crash_ms", System.currentTimeMillis()).commit()
            }
            prev?.uncaughtException(t, e)
        }
    }

    fun lastCrash(ctx: Context, nowMs: Long): Map<String, Any?>? {
        val p = prefs(ctx); val s = p.getString("crash", null) ?: return null
        return mapOf("text" to s, "age_s" to (nowMs - p.getLong("crash_ms", nowMs)) / 1000)
    }

    fun error(ctx: Context, where: String, e: Throwable) {
        prefs(ctx).edit().putString("error", "$where: ${e.javaClass.simpleName}: ${e.message?.take(120)}")
            .putLong("error_ms", System.currentTimeMillis()).apply()
    }

    fun lastError(ctx: Context, nowMs: Long): Map<String, Any?>? {
        val p = prefs(ctx); val s = p.getString("error", null) ?: return null
        return mapOf("text" to s, "age_s" to (nowMs - p.getLong("error_ms", nowMs)) / 1000)
    }

    // ── identity ────────────────────────────────────────────────────────────────────

    /** Account this device publishes for: the sensor-node owner, else the seat account. */
    fun account(ctx: Context): String? {
        val owner = runCatching {
            JSONObject(File(ctx.filesDir, "hone-node/sensor_settings.json").readText()).optString("owner", "")
        }.getOrNull()
        if (!owner.isNullOrBlank()) return owner
        return runCatching { File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrNull()?.takeIf { it.isNotBlank() }
            ?: network.hone.app.account.AccountStore.handle(ctx)   // app account (volunteers without a wallet)
    }

    fun seatAccount(ctx: Context): String? =
        runCatching { File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrNull()?.takeIf { it.isNotBlank() }

    /** This device's seat pubkey (hex), the stable device id source. */
    fun seatPub(ctx: Context): String = Canonical.hex(RoleKeys.devicePub(ctx))

    /** Short stable device id: first 16 hex of the seat pubkey. */
    fun deviceId(ctx: Context): String = seatPub(ctx).take(16)

    class Signer(val keyType: String, internal val seed: ByteArray)

    /**
     * The key public uploads are signed with: the account's posting key when it is on this
     * phone AND matches the chain's posting key for that account; otherwise this device's
     * seat key (the indexer holds seat-signed bird rows back until the seat is bound to the
     * account). The check result is cached for 6 h; with no node reachable the local key is used.
     */
    fun signer(ctx: Context, account: String): Signer {
        val posting = RoleKeys.get(ctx, "posting")
        if (posting != null) {
            val pub = Canonical.hex(Canonical.pubOf(posting))
            if (postingMatches(ctx, account, pub) != false) return Signer("posting", posting)
            skipped(ctx, "posting_key", "phone posting key is not $account's on-chain posting key")
        }
        return Signer("seat", seatSeed(ctx))
    }

    fun postingState(ctx: Context, account: String?): String {
        val posting = RoleKeys.get(ctx, "posting") ?: return "missing"
        if (account == null) return "no account"
        return when (postingMatches(ctx, account, Canonical.hex(Canonical.pubOf(posting)), cachedOnly = true)) {
            true -> "ok"; false -> "mismatch"; null -> "unchecked"
        }
    }

    private fun postingMatches(ctx: Context, account: String, pub: String, cachedOnly: Boolean = false): Boolean? {
        val p = prefs(ctx)
        val key = "pk_${account}_$pub"
        if (System.currentTimeMillis() - p.getLong("${key}_at", 0) < 6 * 3600_000L) return p.getBoolean(key, true)
        if (cachedOnly) return null
        val body = runCatching { ChainHttp.get("/api/account/$account") }.getOrNull() ?: return null
        val chain = runCatching { JSONObject(body).getJSONObject("keys").optString("posting", "") }.getOrNull() ?: return null
        val ok = chain.isEmpty() || chain.equals(pub, ignoreCase = true)
        p.edit().putBoolean(key, ok).putLong("${key}_at", System.currentTimeMillis()).apply()
        return ok
    }

    /** Seat key seed for chain entries the seat signs (SENSOR_SUMMARY). Never logged or exported. */
    fun seatSeedForChain(ctx: Context): ByteArray = seatSeed(ctx)

    private fun seatSeed(ctx: Context): ByteArray {
        return RoleKeys.seatSeed(ctx)
    }

    /** Sign [payload] (a map) canonically and return the envelope JSON string. */
    fun signedBody(ctx: Context, account: String, payload: Map<String, Any?>): String {
        val s = signer(ctx, account)
        return Canonical.encode(Canonical.envelope(Canonical.encode(payload), s.keyType, s.seed))
    }

    // ── HTTP ────────────────────────────────────────────────────────────────────────
    class HttpResult(val code: Int, val body: String)

    /** POST JSON. Returns null when the network is unreachable (retry later). */
    fun post(url: String, json: String): HttpResult? = runCatching {
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"; doOutput = true; connectTimeout = 10_000; readTimeout = 20_000
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("User-Agent", "HONE-app/1.0")
        }
        try {
            c.outputStream.use { it.write(json.toByteArray(Charsets.UTF_8)) }
            val code = c.responseCode
            val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() }.orEmpty()
            HttpResult(code, text)
        } finally { c.disconnect() }
    }.onFailure { Log.w(TAG, "POST $url: ${it.javaClass.simpleName} ${it.message}") }.getOrNull()
}
