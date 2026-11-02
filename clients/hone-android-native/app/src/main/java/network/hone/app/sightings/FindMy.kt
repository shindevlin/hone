package network.hone.app.sightings

import android.content.Context
import android.util.Base64
import android.util.Log
import network.hone.app.data.ChainHttp
import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.json.JSONObject
import java.io.File

/** One decrypted sighting of a device this owner can see (own or shared). */
data class DeviceSighting(
    val device: String, val owner: String, val window: Long, val cellf: String?, val cell: String?,
    val accM: Int?, val rssi: Int, val ts: Long, val source: String,
)

/**
 * Where sealed sightings are found. Today: this phone's own store. Later: an indexer that
 * holds everyone's (lid, window, blob). Same query either way: lids in, blobs out.
 */
interface SightingIndex {
    val name: String
    fun lookup(lids: Collection<String>): List<SightingStore.Enc>
}

class LocalSightingIndex(private val store: SightingStore) : SightingIndex {
    override val name = "this phone"
    override fun lookup(lids: Collection<String>) = store.encSightings(lids)
}
// TODO(indexer): ChainSightingIndex : SightingIndex — POST lids to the indexer.

/**
 * Find my things + location sharing.
 *
 * Own devices: id_seed = HKDF(hide, "hone-id-seed"||device_pub); every window key of the
 * last [days] is recomputed, the lookup ids queried, and each blob opened with sk_w.
 * Shared devices: a grant gives share_k per month; the same window keys follow from it,
 * for the granted months only.
 */
object FindMy {
    private const val TAG = "HoneSight"
    const val INFO_GRANT = "hone-locshare-v1"
    const val INFO_PROVISION = "hone-seed-provision-v1"
    val DEFAULT_GRANTORS = listOf("shindevlin", "natoshisakamoto")

    fun account(ctx: Context): String =
        runCatching { File(ctx.filesDir, "hone-node/witness_account").readText().trim() }.getOrDefault("")

    class Target(val label: String, val owner: String, val shareKeyFor: (Int) -> ByteArray?)

    fun targets(ctx: Context, store: SightingStore): List<Target> {
        val me = account(ctx).ifEmpty { "me" }
        val own = buildList {
            val (seed, _) = RoleKeys.thisSeed(ctx)
            add(Target("This phone", me) { IdKeys.shareKey(seed, it) })
            for ((label, pub) in store.ownedDevices()) {
                val s = runCatching { RoleKeys.seedFor(ctx, pub.unhex()) }.getOrNull() ?: continue
                add(Target(label, me) { IdKeys.shareKey(s, it) })
            }
        }
        val shared = store.sharedDevices().map { sd ->
            val keys = runCatching { JSONObject(sd.keysJson) }.getOrNull() ?: JSONObject()
            Target(sd.label.ifEmpty { sd.device.take(8) }, sd.grantor) { p -> keys.optString(p.toString()).takeIf { it.length == 64 }?.unhex() }
        }
        return own + shared
    }

    fun search(ctx: Context, store: SightingStore, indexes: List<SightingIndex>, days: Int = 7): List<DeviceSighting> {
        val now = IdKeys.window()
        val out = ArrayList<DeviceSighting>()
        for (t in targets(ctx, store)) {
            val keys = HashMap<String, IdKeys.WindowKey>()
            val shareCache = HashMap<Int, ByteArray?>()
            for (w in (now - days * 96L)..now) {
                val sk = shareCache.getOrPut(IdKeys.period(w)) { t.shareKeyFor(IdKeys.period(w)) } ?: continue
                val k = IdKeys.windowKey(sk, w)
                keys[k.lid.hex()] = k
            }
            for (ix in indexes) for (e in ix.lookup(keys.keys)) {
                val k = keys[e.lid] ?: continue
                val pt = IdKeys.open(k.sk, k.pk, SightingsEngine.INFO_SIGHTING, e.blob, aad = k.lid) ?: continue
                val j = runCatching { JSONObject(String(pt)) }.getOrNull() ?: continue
                out += DeviceSighting(t.label, t.owner, j.optLong("w"), j.optString("c").ifEmpty { null }.takeIf { it != "null" },
                    j.optString("cp").ifEmpty { null }.takeIf { it != "null" }, j.optInt("a", -1).takeIf { it >= 0 },
                    j.optInt("r"), j.optLong("t"), ix.name)
            }
        }
        return out.sortedByDescending { it.ts }
    }

    // ── grants (location sharing) ────────────────────────────────────────────────
    fun grantKeyName(grantee: String, deviceHex: String): String =
        "ls1-" + IdKeys.sha256("hone-locshare|$grantee|$deviceHex".toByteArray()).copyOf(16).hex()

    fun grantors(ctx: Context): List<String> = runCatching {
        File(ctx.filesDir, "hone-node/share_from").readLines().map { it.trim() }.filter { it.isNotEmpty() }
    }.getOrNull()?.takeIf { it.isNotEmpty() } ?: DEFAULT_GRANTORS

    fun setGrantors(ctx: Context, list: List<String>) {
        File(ctx.filesDir, "hone-node/share_from").writeText(list.joinToString("\n"))
    }

    /** Pull grants addressed to this account from each grantor's Memory store. A grant
     *  that was deleted (revoked) disappears here on the next refresh. */
    fun refreshShared(ctx: Context, store: SightingStore): Int {
        val memo = RoleKeys.get(ctx, "memo") ?: return 0
        val me = account(ctx); if (me.isEmpty()) return 0
        val xsk = IdKeys.edPrivToX(memo); val xpk = IdKeys.xPub(xsk)
        var n = 0
        for (g in grantors(ctx)) {
            if (g == me) continue
            val body = ChainHttp.get("/api/memory/$g") ?: continue     // offline: keep what we have
            val mem = runCatching { JSONObject(body).getJSONObject("memory") }.getOrNull() ?: continue
            val items = ArrayList<SightingStore.Shared>()
            for (name in mem.keys()) {
                if (!name.startsWith("ls1-")) continue
                val blob = runCatching { Base64.decode(mem.getString(name), Base64.DEFAULT) }.getOrNull() ?: continue
                val pt = IdKeys.open(xsk, xpk, INFO_GRANT, blob, aad = g.toByteArray()) ?: continue
                val j = runCatching { JSONObject(String(pt)) }.getOrNull() ?: continue
                if (j.optString("to") != me || j.optString("from") != g) continue
                if (name != grantKeyName(me, j.optString("device"))) continue
                items += SightingStore.Shared(g, j.optString("device"), j.optString("label"), j.optJSONObject("keys").toString())
            }
            store.replaceShared(g, items); n += items.size
        }
        Log.i(TAG, "shared with me: $n device(s)")
        return n
    }

    /** Build a grant of [devicePub]'s locations for [months] months to [grantee]. */
    fun buildGrant(ctx: Context, grantee: String, devicePub: ByteArray, label: String, months: Int = 2): Pair<String, String> {
        val hide = RoleKeys.get(ctx, "hide") ?: error("import your hide key first")
        val me = account(ctx)
        val memoHex = ChainHttp.get("/api/account/$grantee")?.let { JSONObject(it).optJSONObject("keys")?.optString("memo") }
        require(!memoHex.isNullOrEmpty()) { "$grantee has no memo key on chain" }
        val seed = IdKeys.deviceSeed(hide, devicePub)
        val p0 = IdKeys.period(IdKeys.window())
        val keys = JSONObject().apply { for (p in p0 until p0 + months) put(p.toString(), IdKeys.shareKey(seed, p).hex()) }
        val pt = JSONObject().put("v", 1).put("from", me).put("to", grantee).put("device", devicePub.hex())
            .put("label", label).put("keys", keys).toString()
        val blob = IdKeys.seal(IdKeys.edPubToX(memoHex!!.unhex()), INFO_GRANT, pt.toByteArray(), aad = me.toByteArray())
        return grantKeyName(grantee, devicePub.hex()) to Base64.encodeToString(blob, Base64.NO_WRAP)
    }

    /** Sign + submit MemorySet/MemoryDelete with the imported active key. Returns a status line. */
    fun submitMemory(ctx: Context, key: String, value: String?): String {
        val active = RoleKeys.get(ctx, "active") ?: return "no active key on this phone: run tools/hone_locshare.py on the wallet host"
        val me = account(ctx)
        val nonce = (ChainHttp.get("/api/account/$me")?.let { JSONObject(it).optLong("nonce") } ?: return "no node reachable") + 1
        val fields = sortedMapOf<String, Any>("account" to me, "chain_id" to "hone", "key" to key, "nonce" to nonce,
            "type" to if (value == null) "MEMORY_DELETE" else "MEMORY_SET")
        if (value != null) fields["value"] = value
        val msg = fields.entries.joinToString(",", "{", "}") { (k, v) ->
            JSONObject.quote(k) + ":" + if (v is String) JSONObject.quote(v) else v.toString()
        }
        val sig = Ed25519Signer().run {
            init(true, Ed25519PrivateKeyParameters(active, 0)); val b = msg.toByteArray(); update(b, 0, b.size); generateSignature()
        }.hex()
        val body = JSONObject().put("nonce", nonce).put("signature", sig).apply { if (value != null) put("value", value) }
        val path = "/api/memory/$me/$key"
        val res = if (value != null) ChainHttp.postJson(path, body.toString()) else ChainHttp.deleteJson(path, body.toString())
        return res?.let { "${it.first}: ${it.second.take(120)}" } ?: "no node reachable"
    }

    /** Sealed id_seed for one of the owner's devices (e.g. a Flipper at claim time): the
     *  device opens it with the X25519 form of its ed25519 device key. */
    fun provisionBlob(ctx: Context, devicePub: ByteArray): ByteArray? {
        val seed = RoleKeys.seedFor(ctx, devicePub) ?: return null
        return IdKeys.seal(IdKeys.edPubToX(devicePub), INFO_PROVISION, seed)
    }
}
