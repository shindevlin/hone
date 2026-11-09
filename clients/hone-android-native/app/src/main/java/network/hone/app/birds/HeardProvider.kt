package network.hone.app.birds

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import network.hone.app.account.AccountStore
import network.hone.app.sightings.Cells
import network.hone.app.telemetry.Telemetry

/**
 * Lets Birderlands use Hone's listening and Hone's account on the same phone, so only one app holds the
 * microphone and the player never types an account name. Read only. Guarded by
 * network.hone.permission.READ_HEARD, which the player grants to Birderlands from an Android prompt.
 *
 *   content://<applicationId>.heard/heard?since=<epoch ms>  -> t, name, conf (0..100), cell (H3 res 9 or null)
 *   content://<applicationId>.heard/account                 -> account, handle, referrer
 *
 * Raw coordinates never leave Hone (positions go out as res-9 cells). No audio, no keys.
 */
class HeardProvider : ContentProvider() {
    override fun onCreate() = true

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, sort: String?): Cursor? {
        val ctx = context ?: return null
        return when (uri.lastPathSegment) {
            "heard" -> {
                val since = uri.getQueryParameter("since")?.toLongOrNull() ?: 0L
                MatrixCursor(arrayOf("t", "name", "conf", "cell")).apply {
                    Birds.heard(ctx).asSequence().filter { it.t > since }.take(5000).forEach { h ->
                        val cell = if (h.lat != null && h.lon != null) Cells.cell(h.lat, h.lon, 9) else null
                        addRow(arrayOf<Any?>(h.t, h.name, h.conf, cell))
                    }
                }
            }
            "account" -> MatrixCursor(arrayOf("account", "handle", "referrer")).apply {
                addRow(arrayOf<Any?>(Telemetry.account(ctx), AccountStore.handle(ctx), AccountStore.referrer(ctx)))
            }
            else -> null
        }
    }

    override fun getType(uri: Uri) = "vnd.android.cursor.dir/vnd.hone.${uri.lastPathSegment}"
    override fun insert(uri: Uri, values: ContentValues?) = null
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?) = 0
}
