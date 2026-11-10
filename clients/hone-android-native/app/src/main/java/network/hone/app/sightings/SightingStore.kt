package network.hone.app.sightings

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper

/**
 * Local store (app-private SQLite; nothing here leaves the device except queued public
 * summaries, once the chain's summary commit exists). Schema: docs/SIGHTINGS.md §4.
 *
 * v2 (2026-11-02): rid_sighting (plaintext cells) replaced by enc_sighting (cells sealed to
 * the advertiser's pk_w); owned_device keyed by device pubkey (seeds are derived, never
 * stored); shared_device (grants received); fingerprint (private Wi-Fi positioning).
 */
class SightingStore(ctx: Context) : SQLiteOpenHelper(ctx, "sightings.db", null, 2) {

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("""CREATE TABLE public_summary(cell TEXT NOT NULL, window INTEGER NOT NULL,
            json TEXT NOT NULL, queued INTEGER NOT NULL DEFAULT 1, submitted_at INTEGER,
            PRIMARY KEY(cell, window))""")
        db.execSQL("""CREATE TABLE flipper_frame(dev TEXT NOT NULL, kind TEXT NOT NULL,
            window INTEGER NOT NULL, cell9 TEXT, cellf TEXT, acc REAL,
            rssi INTEGER, ts INTEGER NOT NULL)""")
        db.execSQL("CREATE INDEX flipper_frame_w ON flipper_frame(window)")
        db.execSQL("CREATE TABLE own_router(tag TEXT PRIMARY KEY, label TEXT, registered_at INTEGER)")
        db.execSQL("""CREATE TABLE own_router_cov(tag TEXT NOT NULL, window INTEGER NOT NULL,
            cellf TEXT NOT NULL, rssi INTEGER, PRIMARY KEY(tag, window, cellf))""")
        createV2(db)
    }

    private fun createV2(db: SQLiteDatabase) {
        db.execSQL("""CREATE TABLE enc_sighting(lid TEXT NOT NULL, window INTEGER NOT NULL,
            blob BLOB NOT NULL, ts INTEGER NOT NULL)""")
        db.execSQL("CREATE INDEX enc_sighting_lid ON enc_sighting(lid)")
        db.execSQL("CREATE TABLE owned_device(label TEXT PRIMARY KEY, pub TEXT NOT NULL)")
        db.execSQL("""CREATE TABLE shared_device(grantor TEXT NOT NULL, device TEXT NOT NULL,
            label TEXT, keys TEXT NOT NULL, fetched_at INTEGER, PRIMARY KEY(grantor, device))""")
        db.execSQL("""CREATE TABLE fingerprint(cellf TEXT NOT NULL, tag TEXT NOT NULL,
            rssi_sum REAL NOT NULL, n INTEGER NOT NULL, updated INTEGER,
            PRIMARY KEY(cellf, tag))""")
        db.execSQL("CREATE INDEX fingerprint_tag ON fingerprint(tag)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        if (oldVersion < 2) {
            db.execSQL("DROP TABLE IF EXISTS rid_sighting")
            db.execSQL("DROP TABLE IF EXISTS owned_device")   // held raw seeds (incl. a test seed)
            createV2(db)
        }
    }

    // ── public layer ─────────────────────────────────────────────────────────────
    fun putSummary(cell: String, window: Long, json: String, queue: Boolean) {
        writableDatabase.insertWithOnConflict("public_summary", null, ContentValues().apply {
            put("cell", cell); put("window", window); put("json", json); put("queued", if (queue) 1 else 0)
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun summaryCounts(): Pair<Int, Int> = readableDatabase.rawQuery(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN queued>0 THEN 1 ELSE 0 END),0) FROM public_summary", null,
    ).use { c -> c.moveToFirst(); c.getInt(0) to c.getInt(1) }

    /** Rows not yet handled for the chain, oldest first: (cell, window, json). */
    fun queuedSummaries(limit: Int): List<Triple<String, Long, String>> = readableDatabase.rawQuery(
        "SELECT cell, window, json FROM public_summary WHERE queued=1 ORDER BY window LIMIT ?", arrayOf(limit.toString()),
    ).use { c -> buildList { while (c.moveToNext()) add(Triple(c.getString(0), c.getLong(1), c.getString(2))) } }

    /** Chain handling done: submitted_at = accept time (ms), or a negative marker (map only / refused). */
    fun markSummary(cell: String, window: Long, submittedAt: Long) {
        writableDatabase.update("public_summary", ContentValues().apply { put("queued", 0); put("submitted_at", submittedAt) },
            "cell=? AND window=?", arrayOf(cell, window.toString()))
    }

    /** Keep a row for the chain but out of the queue until the chain accepts its (coarse) cell. */
    fun holdSummary(cell: String, window: Long) {
        writableDatabase.update("public_summary", ContentValues().apply { put("queued", 2) },
            "cell=? AND window=?", arrayOf(cell, window.toString()))
    }

    /** Held rows back into the queue. Returns how many. */
    fun releaseHeld(): Int = writableDatabase.update("public_summary", ContentValues().apply { put("queued", 1) }, "queued=2", null)

    /** One-time: rows the old home rule kept map-only go back to the queue (everything goes on chain now). */
    fun requeueMapOnly(): Int = writableDatabase.update("public_summary", ContentValues().apply { put("queued", 1); putNull("submitted_at") },
        "queued=0 AND submitted_at=? AND cell<>'unlocated'", arrayOf("-1"))

    /** Send everything (2026-11-10): rows once marked map-only (-1) or refused (-2) go back to the queue. */
    fun requeueSendAll(): Int = writableDatabase.update("public_summary", ContentValues().apply { put("queued", 1); putNull("submitted_at") },
        "queued=0 AND submitted_at IN (-1, -2)", null)

    /** A row with no place at all waits (queued=3) for the chain's null-cell gate; never dropped. */
    fun parkSummary(cell: String, window: Long) {
        writableDatabase.update("public_summary", ContentValues().apply { put("queued", 3) },
            "cell=? AND window=?", arrayOf(cell, window.toString()))
    }

    /** Parked rows (no place at all) back into the queue once the chain takes an empty cell. */
    fun releaseParked(): Int = writableDatabase.update("public_summary", ContentValues().apply { put("queued", 1) }, "queued=3", null)

    /** All rows after [rowid] (for the indexer map): (rowid, cell, window, json). */
    fun summariesAfter(rowid: Long, limit: Int): List<SummaryRow> = readableDatabase.rawQuery(
        "SELECT rowid, cell, window, json FROM public_summary WHERE rowid > ? ORDER BY rowid LIMIT ?",
        arrayOf(rowid.toString(), limit.toString()),
    ).use { c -> buildList { while (c.moveToNext()) add(SummaryRow(c.getLong(0), c.getString(1), c.getLong(2), c.getString(3))) } }

    data class SummaryRow(val rowid: Long, val cell: String, val window: Long, val json: String)

    // ── sightings of other HONE devices (sealed) ────────────────────────────────
    fun putEncSighting(lid: String, window: Long, blob: ByteArray, ts: Long) {
        writableDatabase.insert("enc_sighting", null, ContentValues().apply {
            put("lid", lid); put("window", window); put("blob", blob); put("ts", ts)
        })
    }

    class Enc(val lid: String, val window: Long, val blob: ByteArray, val ts: Long)

    fun encSightings(lids: Collection<String>): List<Enc> {
        val out = ArrayList<Enc>()
        for (chunk in lids.chunked(400)) {
            val q = chunk.joinToString(",") { "?" }
            readableDatabase.rawQuery("SELECT lid,window,blob,ts FROM enc_sighting WHERE lid IN ($q)",
                chunk.toTypedArray()).use { c ->
                while (c.moveToNext()) out += Enc(c.getString(0), c.getLong(1), c.getBlob(2), c.getLong(3))
            }
        }
        return out
    }

    fun encCount(): Int = readableDatabase.rawQuery("SELECT COUNT(*) FROM enc_sighting", null)
        .use { it.moveToFirst(); it.getInt(0) }

    // ── flipper frames ───────────────────────────────────────────────────────────
    fun putFlipperFrame(dev: String, kind: String, window: Long, cell9: String?, cellf: String?,
                        acc: Float?, rssi: Int, ts: Long) {
        writableDatabase.insert("flipper_frame", null, ContentValues().apply {
            put("dev", dev); put("kind", kind); put("window", window); put("cell9", cell9); put("cellf", cellf)
            put("acc", acc?.toDouble()); put("rssi", rssi); put("ts", ts)
        })
    }

    // ── owned + shared devices ───────────────────────────────────────────────────
    fun ownedDevices(): List<Pair<String, String>> = readableDatabase.rawQuery(
        "SELECT label, pub FROM owned_device ORDER BY label", null,
    ).use { c -> buildList { while (c.moveToNext()) add(c.getString(0) to c.getString(1)) } }

    fun putOwnedDevice(label: String, pubHex: String) {
        writableDatabase.insertWithOnConflict("owned_device", null, ContentValues().apply {
            put("label", label); put("pub", pubHex)
        }, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun removeOwnedDevice(label: String) { writableDatabase.delete("owned_device", "label=?", arrayOf(label)) }

    class Shared(val grantor: String, val device: String, val label: String, val keysJson: String)

    fun sharedDevices(): List<Shared> = readableDatabase.rawQuery(
        "SELECT grantor, device, label, keys FROM shared_device ORDER BY grantor, label", null,
    ).use { c -> buildList { while (c.moveToNext()) add(Shared(c.getString(0), c.getString(1), c.getString(2) ?: "", c.getString(3))) } }

    /** Replace everything received from [grantor] with [items] (a revoked grant disappears). */
    fun replaceShared(grantor: String, items: List<Shared>) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            db.delete("shared_device", "grantor=?", arrayOf(grantor))
            for (s in items) db.insert("shared_device", null, ContentValues().apply {
                put("grantor", s.grantor); put("device", s.device); put("label", s.label)
                put("keys", s.keysJson); put("fetched_at", System.currentTimeMillis())
            })
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
    }

    // ── own routers ──────────────────────────────────────────────────────────────
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

    // ── Wi-Fi fingerprints (private positioning) ─────────────────────────────────
    fun addFingerprint(cellf: String, obs: Map<String, Int>) {
        val db = writableDatabase
        db.beginTransaction()
        try {
            val now = System.currentTimeMillis()
            for ((tag, rssi) in obs) {
                db.execSQL("INSERT OR IGNORE INTO fingerprint(cellf,tag,rssi_sum,n,updated) VALUES(?,?,0,0,?)",
                    arrayOf(cellf, tag, now))
                db.execSQL("UPDATE fingerprint SET rssi_sum=rssi_sum+?, n=n+1, updated=? WHERE cellf=? AND tag=?",
                    arrayOf(rssi, now, cellf, tag))
            }
            db.setTransactionSuccessful()
        } finally { db.endTransaction() }
    }

    /** cellf -> (tag -> mean rssi) for cells that share at least one tag with [tags]. */
    fun fingerprintsFor(tags: Collection<String>): Map<String, Map<String, Double>> {
        if (tags.isEmpty()) return emptyMap()
        val cells = HashSet<String>()
        for (chunk in tags.chunked(400)) {
            val q = chunk.joinToString(",") { "?" }
            readableDatabase.rawQuery("SELECT DISTINCT cellf FROM fingerprint WHERE tag IN ($q)", chunk.toTypedArray())
                .use { c -> while (c.moveToNext()) cells.add(c.getString(0)) }
        }
        val out = HashMap<String, HashMap<String, Double>>()
        for (chunk in cells.chunked(400)) {
            val q = chunk.joinToString(",") { "?" }
            readableDatabase.rawQuery("SELECT cellf, tag, rssi_sum / n FROM fingerprint WHERE cellf IN ($q)", chunk.toTypedArray())
                .use { c -> while (c.moveToNext()) out.getOrPut(c.getString(0)) { HashMap() }[c.getString(1)] = c.getDouble(2) }
        }
        return out
    }

    fun fingerprintCells(): Int = readableDatabase.rawQuery("SELECT COUNT(DISTINCT cellf) FROM fingerprint", null)
        .use { it.moveToFirst(); it.getInt(0) }

    /** Private-layer retention: 30 days. Fingerprints are kept (they are the map). */
    fun prune(nowWindow: Long) {
        val keep = (nowWindow - 30 * 96).toString()
        val db = writableDatabase
        db.delete("enc_sighting", "window < ?", arrayOf(keep))
        db.delete("flipper_frame", "window < ?", arrayOf(keep))
        db.delete("own_router_cov", "window < ?", arrayOf(keep))
        db.delete("public_summary", "window < ? AND queued=0", arrayOf(keep))   // never drops a row the chain has not had
    }
}
