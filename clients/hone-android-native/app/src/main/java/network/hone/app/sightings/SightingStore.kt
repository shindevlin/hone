package network.hone.app.sightings

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

/**
 * Local store (app-private SQLite, files never leave the device).
 *
 * Public layer:   public_summary  — one row per (cell res 9, window), JSON per
 *                 docs/SIGHTINGS.md. queued=1 rows wait for the chain's summary commit
 *                 (not built yet: nothing is submitted).
 * Private layer:  rid_sighting    — rids of other HONE devices this phone heard (find-my).
 *                 flipper_frame   — Flipper announces/commits heard, location-stamped.
 *                 owned_device    — seeds of the owner's other devices (find-my input).
 *                 own_router      — owner's routers registered by SSID-code proof, keyed
 *                                   by HMAC(K_priv, bssid); the BSSID itself is not stored.
 *                 own_router_cov  — per-window coverage of those routers.
 */
class SightingStore(ctx: Context) : SQLiteOpenHelper(ctx, "sightings.db", null, 1) {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("""CREATE TABLE public_summary(cell TEXT NOT NULL, window INTEGER NOT NULL,
            json TEXT NOT NULL, queued INTEGER NOT NULL DEFAULT 1, submitted_at INTEGER,
            PRIMARY KEY(cell, window))""")
        db.execSQL("""CREATE TABLE rid_sighting(rid TEXT NOT NULL, window INTEGER NOT NULL,
            cell9 TEXT, cellf TEXT, rssi INTEGER, ts INTEGER NOT NULL,
            PRIMARY KEY(rid, window, cellf))""")
        db.execSQL("""CREATE TABLE flipper_frame(dev TEXT NOT NULL, kind TEXT NOT NULL,
            window INTEGER NOT NULL, cell9 TEXT, cellf TEXT, acc REAL,
            rssi INTEGER, ts INTEGER NOT NULL)""")
        db.execSQL("CREATE INDEX flipper_frame_w ON flipper_frame(window)")
        db.execSQL("CREATE TABLE owned_device(label TEXT PRIMARY KEY, seed TEXT NOT NULL)")
        db.execSQL("CREATE TABLE own_router(tag TEXT PRIMARY KEY, label TEXT, registered_at INTEGER)")
        db.execSQL("""CREATE TABLE own_router_cov(tag TEXT NOT NULL, window INTEGER NOT NULL,
            cellf TEXT NOT NULL, rssi INTEGER, PRIMARY KEY(tag, window, cellf))""")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {}

    fun putSummary(cell: String, window: Long, json: String, queue: Boolean) {
        writableDatabase.insertWithOnConflict("public_summary", null, ContentValues().apply {
            put("cell", cell); put("window", window); put("json", json); put("queued", if (queue) 1 else 0)
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun summaryCounts(): Pair<Int, Int> = readableDatabase.rawQuery(
        "SELECT COUNT(*), COALESCE(SUM(queued),0) FROM public_summary", null,
    ).use { c -> c.moveToFirst(); c.getInt(0) to c.getInt(1) }

    fun recentSummaries(limit: Int = 8): List<String> = readableDatabase.rawQuery(
        "SELECT json FROM public_summary ORDER BY window DESC LIMIT ?", arrayOf(limit.toString()),
    ).use { c -> buildList { while (c.moveToNext()) add(c.getString(0)) } }

    // Plain REPLACE (not UPSERT) so it also runs on pre-API-30 SQLite: keeps the latest rssi.
    fun putRidSighting(rid: String, window: Long, cell9: String?, cellf: String?, rssi: Int, ts: Long) {
        writableDatabase.insertWithOnConflict("rid_sighting", null, ContentValues().apply {
            put("rid", rid); put("window", window); put("cell9", cell9); put("cellf", cellf ?: "")
            put("rssi", rssi); put("ts", ts)
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    data class Sighting(val rid: String, val window: Long, val cell9: String?, val cellf: String?, val rssi: Int, val ts: Long)

    fun ridSightings(rids: Collection<String>): List<Sighting> {
        val out = ArrayList<Sighting>()
        for (chunk in rids.chunked(400)) {
            val q = chunk.joinToString(",") { "?" }
            readableDatabase.rawQuery(
                "SELECT rid,window,cell9,cellf,rssi,ts FROM rid_sighting WHERE rid IN ($q)", chunk.toTypedArray(),
            ).use { c ->
                while (c.moveToNext()) out += Sighting(c.getString(0), c.getLong(1), c.getString(2),
                    c.getString(3), c.getInt(4), c.getLong(5))
            }
        }
        return out.sortedByDescending { it.ts }
    }

    fun putFlipperFrame(dev: String, kind: String, window: Long, cell9: String?, cellf: String?,
                        acc: Float?, rssi: Int, ts: Long) {
        writableDatabase.insert("flipper_frame", null, ContentValues().apply {
            put("dev", dev); put("kind", kind); put("window", window); put("cell9", cell9); put("cellf", cellf)
            put("acc", acc?.toDouble()); put("rssi", rssi); put("ts", ts)
        })
    }

    fun flipperFrameCount(): Int = readableDatabase.rawQuery("SELECT COUNT(*) FROM flipper_frame", null)
        .use { it.moveToFirst(); it.getInt(0) }

    fun ownedDevices(): List<Pair<String, String>> = readableDatabase.rawQuery(
        "SELECT label, seed FROM owned_device ORDER BY label", null,
    ).use { c -> buildList { while (c.moveToNext()) add(c.getString(0) to c.getString(1)) } }

    fun putOwnedDevice(label: String, seedHex: String) {
        writableDatabase.insertWithOnConflict("owned_device", null, ContentValues().apply {
            put("label", label); put("seed", seedHex)
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun removeOwnedDevice(label: String) { writableDatabase.delete("owned_device", "label=?", arrayOf(label)) }

    fun ownRouterTags(): Set<String> = readableDatabase.rawQuery("SELECT tag FROM own_router", null)
        .use { c -> buildSet { while (c.moveToNext()) add(c.getString(0)) } }

    fun registerRouter(tag: String, label: String) {
        writableDatabase.insertWithOnConflict("own_router", null, ContentValues().apply {
            put("tag", tag); put("label", label); put("registered_at", System.currentTimeMillis())
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun putRouterCoverage(tag: String, window: Long, cellf: String, rssi: Int) {
        writableDatabase.insertWithOnConflict("own_router_cov", null, ContentValues().apply {
            put("tag", tag); put("window", window); put("cellf", cellf); put("rssi", rssi)
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun routerCoverageCount(): Int = readableDatabase.rawQuery("SELECT COUNT(*) FROM own_router_cov", null)
        .use { it.moveToFirst(); it.getInt(0) }

    /** Private-layer retention: 30 days. Unsubmitted public summaries are kept 30 days too. */
    fun prune(nowWindow: Long) {
        val keep = (nowWindow - 30 * 96).toString()
        val db = writableDatabase
        db.delete("rid_sighting", "window < ?", arrayOf(keep))
        db.delete("flipper_frame", "window < ?", arrayOf(keep))
        db.delete("own_router_cov", "window < ?", arrayOf(keep))
        db.delete("public_summary", "window < ?", arrayOf(keep))
    }
}
