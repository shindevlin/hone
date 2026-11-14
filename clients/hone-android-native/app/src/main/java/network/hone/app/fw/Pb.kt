package network.hone.app.fw

import java.io.ByteArrayOutputStream

/** Minimal protobuf (proto3) writer/reader for the Flipper RPC messages we use. Pure Kotlin. */
object Pb {
    class W {
        private val o = ByteArrayOutputStream()
        fun varint(v: Long): W { var x = v; while (true) { if (x and 0x7F.inv().toLong() == 0L) { o.write(x.toInt()); return this }; o.write(((x and 0x7F) or 0x80).toInt()); x = x ushr 7 } }
        private fun tag(f: Int, wt: Int) = varint(((f shl 3) or wt).toLong())
        fun u(f: Int, v: Long): W { if (v != 0L) { tag(f, 0); varint(v) }; return this }
        fun b(f: Int, v: Boolean): W = if (v) u(f, 1) else this
        fun bytes(f: Int, v: ByteArray): W { tag(f, 2); varint(v.size.toLong()); o.write(v); return this }
        fun str(f: Int, v: String): W = if (v.isEmpty()) this else bytes(f, v.toByteArray(Charsets.UTF_8))
        fun msg(f: Int, m: W): W = bytes(f, m.toBytes())
        fun toBytes(): ByteArray = o.toByteArray()
    }

    /** Field number -> list of values (Long for varints, ByteArray for length-delimited). */
    fun read(b: ByteArray): Map<Int, List<Any>> {
        val out = LinkedHashMap<Int, MutableList<Any>>()
        var i = 0
        fun vr(): Long { var s = 0; var r = 0L; while (true) { val x = b[i++].toInt() and 0xFF; r = r or ((x and 0x7F).toLong() shl s); if (x < 0x80) return r; s += 7 } }
        while (i < b.size) {
            val t = vr(); val f = (t ushr 3).toInt()
            val v: Any = when ((t and 7).toInt()) {
                0 -> vr()
                2 -> { val n = vr().toInt(); b.copyOfRange(i, i + n).also { i += n } }
                1 -> { i += 8; 0L }
                5 -> { i += 4; 0L }
                else -> error("unsupported wire type")
            }
            out.getOrPut(f) { ArrayList() }.add(v)
        }
        return out
    }

    /** varint length prefix (the RPC stream framing). */
    fun delimited(m: ByteArray): ByteArray = W().varint(m.size.toLong()).toBytes() + m

    /** Splits a stream buffer into complete delimited messages; returns (messages, bytes consumed). */
    fun split(buf: ByteArray): Pair<List<ByteArray>, Int> {
        val out = ArrayList<ByteArray>(); var i = 0
        while (i < buf.size) {
            var s = 0; var n = 0L; var j = i; var done = false
            while (j < buf.size) { val x = buf[j++].toInt() and 0xFF; n = n or ((x and 0x7F).toLong() shl s); if (x < 0x80) { done = true; break }; s += 7 }
            if (!done || j + n > buf.size) break
            out.add(buf.copyOfRange(j, j + n.toInt())); i = j + n.toInt()
        }
        return out to i
    }
}
