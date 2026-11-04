package network.hone.app.account

import android.content.Context
import network.hone.app.sightings.RoleKeys
import network.hone.app.telemetry.Canonical
import network.hone.app.telemetry.Telemetry
import network.hone.app.telemetry.Vault
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * App account (email + password), served by the HONE indexer. It holds the app profile
 * only (name, life list, settings, devices); nothing about it goes on chain and the email
 * never leaves the indexer. This phone's device key (sealed in the Keystore) is linked to
 * the account, and the account to whoever invited the user ("none" = community pool).
 */
object AccountStore {
    const val API = "https://honemesh.net/api/accounts"
    private const val SESSION = "hone.account.session"

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_account", Context.MODE_PRIVATE)
    private fun sessionFile(ctx: Context) = File(ctx.filesDir, "account/session.bin").apply { parentFile?.mkdirs() }

    fun handle(ctx: Context): String? = prefs(ctx).getString("handle", null)
    fun email(ctx: Context): String? = prefs(ctx).getString("email", null)
    fun name(ctx: Context): String? = prefs(ctx).getString("name", null)?.takeIf { it.isNotBlank() }
    fun verified(ctx: Context) = prefs(ctx).getBoolean("verified", false)
    fun signedIn(ctx: Context) = handle(ctx) != null && token(ctx) != null
    /** The inviting account, or null for the community pool. */
    fun referrer(ctx: Context): String? = prefs(ctx).getString("ref", null)?.takeIf { it.isNotBlank() && it != "none" }

    fun pendingRef(ctx: Context): String? = prefs(ctx).getString("pending_ref", null)
    fun setPendingRef(ctx: Context, ref: String) {
        val r = ref.trim().lowercase()
        if (r.matches(Regex("[a-z0-9][a-z0-9._-]{0,31}"))) prefs(ctx).edit().putString("pending_ref", r).apply()
    }

    /** First run finished (or skipped) on this phone. */
    fun onboarded(ctx: Context) = prefs(ctx).getBoolean("onboarded", false)
    fun setOnboarded(ctx: Context) = prefs(ctx).edit().putBoolean("onboarded", true).apply()

    private fun token(ctx: Context): String? = runCatching {
        sessionFile(ctx).takeIf { it.exists() }?.let { Vault.open(SESSION, it.readBytes())?.let { b -> String(b) } }
    }.getOrNull()

    private fun saveSession(ctx: Context, j: JSONObject) {
        j.optString("token").takeIf { it.isNotEmpty() }?.let { sessionFile(ctx).writeBytes(Vault.seal(SESSION, it.toByteArray())) }
        saveProfile(ctx, j.optJSONObject("user") ?: j)
    }

    private fun saveProfile(ctx: Context, u: JSONObject) {
        val e = prefs(ctx).edit()
        u.optString("handle").takeIf { it.isNotEmpty() }?.let { e.putString("handle", it) }
        u.optString("email").takeIf { it.isNotEmpty() }?.let { e.putString("email", it) }
        if (u.has("name")) e.putString("name", u.optString("name"))
        if (u.has("ref")) e.putString("ref", u.optString("ref"))
        if (u.has("verified")) e.putBoolean("verified", u.optBoolean("verified"))
        e.apply()
    }

    class Failure(message: String) : Exception(message)

    /** Proof that this phone holds its device key, for linking the device to the account. */
    private fun seatProof(ctx: Context, email: String): JSONObject {
        val seed = RoleKeys.seatSeed(ctx)
        val payload = Canonical.encode(mapOf("kind" to "seat_link", "seat_pub" to Canonical.hex(Canonical.pubOf(seed)),
            "email_sha" to sha(email.trim().lowercase()), "label" to android.os.Build.MODEL, "ts" to System.currentTimeMillis() / 1000))
        return JSONObject(Canonical.envelope(payload, "seat", seed))
    }

    private fun sha(s: String) = MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

    private fun call(ctx: Context, method: String, path: String, body: JSONObject? = null, auth: Boolean = false): JSONObject {
        val c = (URL(API + path).openConnection() as HttpURLConnection).apply {
            requestMethod = method; connectTimeout = 10_000; readTimeout = 20_000
            setRequestProperty("Content-Type", "application/json"); setRequestProperty("User-Agent", "HONE-app/1.0")
            if (auth) token(ctx)?.let { setRequestProperty("Authorization", "Bearer $it") }
            if (body != null) doOutput = true
        }
        try {
            body?.let { b -> c.outputStream.use { it.write(b.toString().toByteArray()) } }
            val code = c.responseCode
            val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() }.orEmpty()
            val j = runCatching { JSONObject(text) }.getOrDefault(JSONObject())
            if (code >= 400) throw Failure(j.optString("error").ifEmpty { "Something went wrong ($code). Try again." })
            return j
        } catch (e: java.io.IOException) {
            throw Failure("No connection. Check your internet and try again.")
        } finally { c.disconnect() }
    }

    fun signup(ctx: Context, name: String, email: String, password: String, ref: String?, consentText: String) {
        val r = ref?.trim()?.lowercase()?.ifEmpty { null } ?: "none"
        val j = call(ctx, "POST", "/signup", JSONObject().put("name", name.trim()).put("email", email.trim()).put("password", password)
            .put("ref", r).put("consent", true).put("consent_text", consentText).put("seat", seatProof(ctx, email)))
        saveSession(ctx, j)
    }

    fun login(ctx: Context, email: String, password: String) {
        saveSession(ctx, call(ctx, "POST", "/login", JSONObject().put("email", email.trim()).put("password", password).put("seat", seatProof(ctx, email))))
    }

    fun verify(ctx: Context, code: String) { saveProfile(ctx, call(ctx, "POST", "/verify", JSONObject().put("code", code.trim()), auth = true)) }

    fun resend(ctx: Context) { call(ctx, "POST", "/resend", JSONObject(), auth = true) }

    /** Profile from the server (devices included); also refreshes the cached fields. */
    fun me(ctx: Context): JSONObject = call(ctx, "GET", "/me", auth = true).also { saveProfile(ctx, it) }

    /** Push the life list and settings (no audio, no places: species and first-heard times only). */
    fun syncProfile(ctx: Context) {
        if (!signedIn(ctx)) return
        val life = JSONArray()
        network.hone.app.engage.Engagement.lifeList(ctx).forEach { (latin, t) -> life.put(JSONObject().put("latin", latin).put("first_ms", t)) }
        val settings = JSONObject().put("digest", network.hone.app.engage.Engagement.digestEnabled(ctx))
            .put("show_name", ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE).getBoolean("show_name", false))
        runCatching { call(ctx, "PUT", "/me", JSONObject().put("life_list", life).put("settings", settings), auth = true) }
            .onFailure { Telemetry.upload(ctx, "account", false, it.message) }
    }

    fun logout(ctx: Context) {
        runCatching { call(ctx, "POST", "/logout", JSONObject(), auth = true) }
        sessionFile(ctx).delete()
        prefs(ctx).edit().remove("handle").remove("email").remove("verified").remove("name").apply()
    }
}
