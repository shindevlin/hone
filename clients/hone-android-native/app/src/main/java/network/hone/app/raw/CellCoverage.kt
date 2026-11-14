package network.hone.app.raw

import android.annotation.SuppressLint
import android.content.Context
import android.os.Build
import android.telephony.CellInfo
import android.telephony.CellInfoCdma
import android.telephony.CellInfoGsm
import android.telephony.CellInfoLte
import android.telephony.CellInfoNr
import android.telephony.CellInfoWcdma
import android.telephony.CellIdentityNr
import android.telephony.CellSignalStrengthNr
import android.telephony.TelephonyManager

/**
 * Cell coverage (Josh 2026-11-14): every cell the modem reports, as "cell:<token>" with signal in
 * dBm (RSRP for LTE/NR, RSCP/dBm for UMTS, dBm for GSM/CDMA) and attrs rat, op (MCC-MNC), reg
 * (serving), rsrq, sinr. The cell identity is hashed with its operator; never sent in the clear.
 * Sampled at most once a minute; the change encoder reports moves past 6 dB and a forced
 * sample every 15 minutes.
 */
object CellCoverage {
    private const val UNAVAILABLE = Int.MAX_VALUE
    private fun ok(v: Int) = v != UNAVAILABLE
    /** MCC+MNC strings exist from API 28; older phones fall back to the registered operator. */
    private fun opOf(mcc: () -> String?, mnc: () -> String?): String? =
        if (Build.VERSION.SDK_INT >= 28) runCatching { "${mcc() ?: ""}${mnc() ?: ""}" }.getOrNull() else null

    /** (key, dBm, attrs) for every reported cell, or null when the phone gives nothing (no SIM, no permission). */
    @SuppressLint("MissingPermission")
    fun read(ctx: Context): List<Triple<String, Double, Map<String, Any?>>>? {
        val tm = ctx.getSystemService(TelephonyManager::class.java) ?: return null
        val cells = runCatching { tm.allCellInfo }.getOrNull() ?: return null
        val homeOp = runCatching { tm.networkOperator }.getOrNull().orEmpty()
        val out = ArrayList<Triple<String, Double, Map<String, Any?>>>()
        for (c in cells) {
            val a = LinkedHashMap<String, Any?>()
            a["reg"] = if (c.isRegistered) 1 else 0
            var id: String? = null; var op: String? = null; var dbm: Int? = null
            when (c) {
                is CellInfoLte -> {
                    a["rat"] = "LTE"; val i = c.cellIdentity; val s = c.cellSignalStrength
                    op = opOf({ i.mccString }, { i.mncString }); if (ok(i.ci)) id = "${i.ci}" else if (ok(i.pci)) id = "pci${i.pci}:${i.earfcn}"
                    dbm = s.rsrp.takeIf { ok(it) }; if (ok(s.rsrq)) a["rsrq"] = s.rsrq; if (ok(s.rssnr)) a["sinr"] = s.rssnr
                    if (ok(i.earfcn)) a["arfcn"] = i.earfcn
                }
                is CellInfoGsm -> {
                    a["rat"] = "GSM"; val i = c.cellIdentity
                    op = opOf({ i.mccString }, { i.mncString }); if (ok(i.cid)) id = "${i.lac}:${i.cid}"
                    dbm = c.cellSignalStrength.dbm.takeIf { ok(it) }
                }
                is CellInfoWcdma -> {
                    a["rat"] = "UMTS"; val i = c.cellIdentity
                    op = opOf({ i.mccString }, { i.mncString }); if (ok(i.cid)) id = "${i.lac}:${i.cid}"
                    dbm = c.cellSignalStrength.dbm.takeIf { ok(it) }
                }
                is CellInfoCdma -> {
                    a["rat"] = "CDMA"; val i = c.cellIdentity
                    id = "${i.systemId}:${i.networkId}:${i.basestationId}"
                    dbm = c.cellSignalStrength.dbm.takeIf { ok(it) }
                }
                else -> if (Build.VERSION.SDK_INT >= 29 && c is CellInfoNr) {
                    a["rat"] = "NR"; val i = c.cellIdentity as CellIdentityNr; val s = c.cellSignalStrength as CellSignalStrengthNr
                    op = opOf({ i.mccString }, { i.mncString })
                    if (i.nci != Long.MAX_VALUE) id = "${i.nci}" else if (ok(i.pci)) id = "pci${i.pci}:${i.nrarfcn}"
                    dbm = s.ssRsrp.takeIf { ok(it) }; if (ok(s.ssRsrq)) a["rsrq"] = s.ssRsrq; if (ok(s.ssSinr)) a["sinr"] = s.ssSinr
                    if (ok(i.nrarfcn)) a["arfcn"] = i.nrarfcn
                }
            }
            if (dbm == null || dbm > 0 || dbm < -160) continue
            val operator = op?.takeIf { it.length >= 5 } ?: homeOp.takeIf { c.isRegistered && it.length >= 5 }
            operator?.let { a["op"] = it }
            val key = "cell:" + RawFmt.token("cell", operator ?: "?", a["rat"].toString(), id ?: "anon:${a["arfcn"]}")
            out.add(Triple(key, dbm.toDouble(), a))
        }
        return out
    }
}
