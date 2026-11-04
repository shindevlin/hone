package network.hone.app.station

import java.io.BufferedInputStream
import java.io.Closeable
import java.io.EOFException
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.security.MessageDigest
import java.security.SecureRandom

/**
 * Minimal RTSP client that sets up ONLY the audio track of a camera stream, with RTP
 * interleaved on the RTSP TCP connection (works through home routers, no UDP ports).
 * The video track in the SDP is never SETUP, so the camera never sends video to the phone:
 * nothing to fetch, decode or keep. Basic and Digest auth. Pure Kotlin (JVM-tested).
 * Credentials go only into Authorization headers; URLs and messages never carry them.
 */
class RtspException(val code: Int, message: String) : Exception(message)

data class SdpMedia(
    val type: String, val payloadType: Int, val encoding: String, val clockRate: Int,
    val channels: Int, val control: String?, val fmtp: Map<String, String>,
)

object Sdp {
    private val STATIC = mapOf(
        0 to Triple("PCMU", 8000, 1), 8 to Triple("PCMA", 8000, 1),
        10 to Triple("L16", 44100, 2), 11 to Triple("L16", 44100, 1),
    )
    val SUPPORTED = setOf("PCMA", "PCMU", "L16", "MPEG4-GENERIC")

    /** (session-level control, media sections). */
    fun parse(sdp: String): Pair<String?, List<SdpMedia>> {
        var sessionControl: String? = null
        val out = ArrayList<SdpMedia>()
        var type: String? = null
        var pt = -1; var enc = ""; var rate = 0; var ch = 1
        var control: String? = null
        var fmtp = emptyMap<String, String>()
        fun flush() { type?.let { out += SdpMedia(it, pt, enc.uppercase(), rate, ch, control, fmtp) } }
        for (raw in sdp.lines()) {
            val l = raw.trim()
            when {
                l.startsWith("m=") -> {
                    flush()
                    val p = l.substring(2).split(" ")
                    type = p[0]; pt = p.getOrNull(3)?.toIntOrNull() ?: -1
                    val s = STATIC[pt]; enc = s?.first ?: ""; rate = s?.second ?: 0; ch = s?.third ?: 1
                    control = null; fmtp = emptyMap()
                }
                l.startsWith("a=control:") -> {
                    val c = l.substringAfter("a=control:").trim()
                    if (type == null) sessionControl = c else control = c
                }
                l.startsWith("a=rtpmap:") && type != null -> {
                    val parts = l.substringAfter("a=rtpmap:").split(" ", limit = 2)
                    if (parts[0].toIntOrNull() == pt) {
                        val q = parts.getOrElse(1) { "" }.split("/")
                        enc = q[0]; rate = q.getOrNull(1)?.toIntOrNull() ?: rate; ch = q.getOrNull(2)?.toIntOrNull() ?: 1
                    }
                }
                l.startsWith("a=fmtp:") && type != null -> {
                    val parts = l.substringAfter("a=fmtp:").split(" ", limit = 2)
                    if (parts[0].toIntOrNull() == pt) fmtp = parts.getOrElse(1) { "" }.split(";").mapNotNull { kv ->
                        val i = kv.indexOf('=')
                        if (i <= 0) null else kv.substring(0, i).trim().lowercase() to kv.substring(i + 1).trim()
                    }.toMap()
                }
            }
        }
        flush()
        return sessionControl to out
    }

    fun pickAudio(media: List<SdpMedia>): SdpMedia? =
        media.filter { it.type == "audio" }.firstOrNull { it.encoding in SUPPORTED }
}

/** Basic or Digest (RFC 2617, qop=auth or none) for RTSP. */
internal class RtspAuth(private val user: String?, private val pass: String?) {
    private var scheme: String? = null
    private var realm = ""; private var nonce = ""; private var qop: String? = null; private var opaque: String? = null
    private var nc = 0
    val hasCreds get() = !user.isNullOrEmpty()

    /** Takes the WWW-Authenticate headers of a 401. Returns false when retrying cannot help. */
    fun accept(headers: List<String>): Boolean {
        val digest = headers.firstOrNull { it.trim().startsWith("Digest", ignoreCase = true) }
        if (digest != null) {
            val p = params(digest.trim().substringAfter(" "))
            val newNonce = p["nonce"] ?: return false
            val stale = p["stale"].equals("true", ignoreCase = true)
            if (scheme == "Digest" && newNonce == nonce && !stale) return false   // same challenge: wrong password
            scheme = "Digest"; realm = p["realm"] ?: ""; nonce = newNonce; opaque = p["opaque"]
            qop = p["qop"]?.split(",")?.map { it.trim() }?.firstOrNull { it == "auth" }; nc = 0
            return true
        }
        if (headers.any { it.trim().startsWith("Basic", ignoreCase = true) } && scheme != "Basic") { scheme = "Basic"; return true }
        return false
    }

    fun header(method: String, uri: String): String? {
        if (!hasCreds) return null
        return when (scheme) {
            "Basic" -> "Basic " + java.util.Base64.getEncoder().encodeToString("$user:$pass".toByteArray())
            "Digest" -> {
                val ha1 = md5("$user:$realm:$pass"); val ha2 = md5("$method:$uri")
                val sb = StringBuilder("Digest username=\"$user\", realm=\"$realm\", nonce=\"$nonce\", uri=\"$uri\"")
                if (qop != null) {
                    nc++
                    val ncs = "%08x".format(nc)
                    val cnonce = ByteArray(8).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }
                    sb.append(", response=\"${md5("$ha1:$nonce:$ncs:$cnonce:$qop:$ha2")}\", qop=$qop, nc=$ncs, cnonce=\"$cnonce\"")
                } else sb.append(", response=\"${md5("$ha1:$nonce:$ha2")}\"")
                opaque?.let { sb.append(", opaque=\"$it\"") }
                sb.toString()
            }
            else -> null
        }
    }

    companion object {
        fun md5(s: String) = MessageDigest.getInstance("MD5").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }
        fun params(s: String): Map<String, String> =
            Regex("(\\w+)=(?:\"([^\"]*)\"|([^,\\s]*))").findAll(s).associate { m ->
                m.groupValues[1].lowercase() to (m.groupValues[2].ifEmpty { m.groupValues[3] })
            }
    }
}

object Rtp {
    /** (payload, RTP timestamp, marker) of one RTP packet, or null if malformed. */
    fun payload(p: ByteArray): Triple<ByteArray, Long, Boolean>? {
        if (p.size < 12 || (p[0].toInt() and 0xC0) != 0x80) return null
        val cc = p[0].toInt() and 0x0F
        val ext = (p[0].toInt() and 0x10) != 0
        val pad = (p[0].toInt() and 0x20) != 0
        val marker = (p[1].toInt() and 0x80) != 0
        val ts = ((p[4].toLong() and 0xFF) shl 24) or ((p[5].toLong() and 0xFF) shl 16) or
            ((p[6].toLong() and 0xFF) shl 8) or (p[7].toLong() and 0xFF)
        var off = 12 + 4 * cc
        if (ext) {
            if (p.size < off + 4) return null
            off += 4 + 4 * (((p[off + 2].toInt() and 0xFF) shl 8) or (p[off + 3].toInt() and 0xFF))
        }
        var end = p.size
        if (pad) end -= p[p.size - 1].toInt() and 0xFF
        if (off > end) return null
        return Triple(p.copyOfRange(off, end), ts, marker)
    }

    /** RFC 3640 (AAC-hbr): AU-header section, then the access units. */
    fun aacUnits(payload: ByteArray, sizeLength: Int = 13, indexLength: Int = 3): List<ByteArray> {
        if (payload.size < 2) return emptyList()
        val hdrBits = ((payload[0].toInt() and 0xFF) shl 8) or (payload[1].toInt() and 0xFF)
        val per = sizeLength + indexLength
        if (per <= 0 || hdrBits == 0) return emptyList()
        var bitPos = 16
        var dataOff = 2 + (hdrBits + 7) / 8
        val out = ArrayList<ByteArray>()
        repeat(hdrBits / per) {
            if ((bitPos + sizeLength + 7) / 8 > payload.size) return out
            val size = bits(payload, bitPos, sizeLength); bitPos += per
            if (size <= 0 || dataOff + size > payload.size) return out
            out += payload.copyOfRange(dataOff, dataOff + size); dataOff += size
        }
        return out
    }

    private fun bits(b: ByteArray, start: Int, n: Int): Int {
        var v = 0
        for (i in 0 until n) { val bit = start + i; v = (v shl 1) or ((b[bit / 8].toInt() shr (7 - bit % 8)) and 1) }
        return v
    }

    /** AAC AudioSpecificConfig (hex from fmtp config=) -> (sample rate, channels). */
    fun aacConfig(hex: String): Pair<Int, Int>? {
        val b = runCatching { hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray() }.getOrNull() ?: return null
        if (b.size < 2) return null
        val idx = ((b[0].toInt() and 0x07) shl 1) or ((b[1].toInt() and 0x80) shr 7)
        val ch = (b[1].toInt() and 0x78) shr 3
        val rates = intArrayOf(96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350)
        return (rates.getOrNull(idx) ?: return null) to ch.coerceAtLeast(1)
    }
}

class RtspAudioClient(
    private val host: String, private val port: Int, path: String,
    user: String?, pass: String?, private val timeoutMs: Int = 8_000,
) : Closeable {
    private val auth = RtspAuth(user, pass)
    private val path = if (path.startsWith("/")) path else "/$path"
    private var sock: Socket? = null
    private lateinit var input: BufferedInputStream
    private lateinit var out: OutputStream
    private var cseq = 0
    private var session: String? = null
    private var sessionTimeoutS = 60
    private var channel = 0
    private var playUrl = ""
    @Volatile var bytesIn = 0L; private set
    lateinit var media: SdpMedia; private set
    var videoTracksSkipped = 0; private set

    /** Stream URL without credentials (safe to show). */
    val url: String get() = "rtsp://$host:$port$path"

    class Response(val code: Int, val headers: Map<String, List<String>>, val body: String) {
        fun h(n: String) = headers[n.lowercase()]?.firstOrNull()
    }

    /** DESCRIBE, SETUP of the audio track only, PLAY. Returns the audio track. */
    fun open(): SdpMedia {
        val s = Socket()
        s.connect(InetSocketAddress(host, port), timeoutMs)
        s.soTimeout = timeoutMs; s.tcpNoDelay = true
        sock = s; input = BufferedInputStream(s.getInputStream(), 65536); out = s.getOutputStream()
        val d = request("DESCRIBE", url, mapOf("Accept" to "application/sdp"))
        val base0 = d.h("content-base") ?: d.h("content-location") ?: url
        val base = if (base0.endsWith("/")) base0 else "$base0/"
        val (sessCtl, medias) = Sdp.parse(d.body)
        videoTracksSkipped = medias.count { it.type != "audio" }
        val audio = medias.filter { it.type == "audio" }
        val a = Sdp.pickAudio(medias) ?: throw RtspException(415,
            if (audio.isEmpty()) "this camera stream has no audio track"
            else "audio codec not supported: ${audio.joinToString { it.encoding }}")
        media = a
        val st = request("SETUP", resolve(base, a.control), mapOf("Transport" to "RTP/AVP/TCP;unicast;interleaved=0-1"))
        st.h("session")?.let { v ->
            session = v.substringBefore(';').trim()
            Regex("timeout=(\\d+)").find(v)?.let { sessionTimeoutS = it.groupValues[1].toInt() }
        }
        st.h("transport")?.let { t -> Regex("interleaved=(\\d+)").find(t)?.let { channel = it.groupValues[1].toInt() } }
        playUrl = if (sessCtl != null && sessCtl != "*") resolve(base, sessCtl) else base0
        request("PLAY", playUrl, mapOf("Range" to "npt=0.000-"))
        return a
    }

    private fun resolve(base: String, ctl: String?): String = when {
        ctl == null || ctl == "*" -> base.trimEnd('/')
        ctl.startsWith("rtsp://", ignoreCase = true) -> ctl
        else -> base + ctl.trimStart('/')
    }

    @Synchronized
    private fun send(method: String, uri: String, extra: Map<String, String>) {
        val sb = StringBuilder("$method $uri RTSP/1.0\r\nCSeq: ${++cseq}\r\nUser-Agent: HONE-station/1\r\n")
        session?.let { sb.append("Session: $it\r\n") }
        auth.header(method, uri)?.let { sb.append("Authorization: $it\r\n") }
        extra.forEach { (k, v) -> sb.append("$k: $v\r\n") }
        sb.append("\r\n")
        out.write(sb.toString().toByteArray()); out.flush()
    }

    private fun request(method: String, uri: String, extra: Map<String, String>): Response {
        send(method, uri, extra)
        var r = readResponse()
        if (r.code == 401) {
            if (!auth.hasCreds) throw RtspException(401, "the camera needs a username and password")
            if (auth.accept(r.headers["www-authenticate"].orEmpty())) { send(method, uri, extra); r = readResponse() }
            if (r.code == 401) throw RtspException(401, "wrong username or password")
        }
        if (r.code != 200) throw RtspException(r.code, "$method failed (${r.code})")
        return r
    }

    private fun readLine(first: Int): String {
        val sb = StringBuilder()
        var c = first
        while (c != -1 && c != '\n'.code) { if (c != '\r'.code) sb.append(c.toChar()); c = input.read() }
        if (c == -1 && sb.isEmpty()) throw EOFException("camera closed the connection")
        return sb.toString()
    }

    private fun readHeaders(status: String): Response {
        val code = status.split(" ").getOrNull(1)?.toIntOrNull() ?: throw RtspException(0, "bad RTSP reply")
        val h = HashMap<String, MutableList<String>>()
        while (true) {
            val l = readLine(input.read())
            if (l.isEmpty()) break
            val i = l.indexOf(':')
            if (i > 0) h.getOrPut(l.substring(0, i).trim().lowercase()) { ArrayList() }.add(l.substring(i + 1).trim())
        }
        val n = h["content-length"]?.firstOrNull()?.toIntOrNull() ?: 0
        return Response(code, h, if (n > 0) String(readN(n)) else "")
    }

    private fun readResponse(): Response {
        while (true) {
            val b = input.read()
            if (b == -1) throw EOFException("camera closed the connection")
            if (b == '$'.code) { readFrame(); continue }        // media that arrived before the reply
            val status = readLine(b)
            if (status.startsWith("RTSP/")) return readHeaders(status)
        }
    }

    private fun readFrame(): Pair<Int, ByteArray> {
        val ch = input.read(); val hi = input.read(); val lo = input.read()
        if (lo < 0) throw EOFException("camera closed the connection")
        return ch to readN((hi shl 8) or lo)
    }

    private fun readN(n: Int): ByteArray {
        val b = ByteArray(n); var o = 0
        while (o < n) { val r = input.read(b, o, n - o); if (r < 0) throw EOFException("camera closed the connection"); o += r }
        bytesIn += n
        return b
    }

    /** Reads audio RTP until [stop] is true, keeping the session alive with OPTIONS. */
    fun stream(stop: () -> Boolean, onRtp: (ByteArray, Long, Boolean) -> Unit) {
        var lastKeep = System.currentTimeMillis()
        val keepMs = (sessionTimeoutS * 1000L / 2).coerceIn(10_000L, 30_000L)
        while (!stop()) {
            val now = System.currentTimeMillis()
            if (now - lastKeep > keepMs) { send("OPTIONS", playUrl, emptyMap()); lastKeep = now }
            val b = input.read()
            if (b == -1) throw EOFException("camera closed the connection")
            if (b == '$'.code) {
                val (ch, data) = readFrame()
                if (ch == channel) Rtp.payload(data)?.let { onRtp(it.first, it.second, it.third) }
            } else {
                val status = readLine(b)
                if (status.startsWith("RTSP/")) readHeaders(status)    // keep-alive reply
            }
        }
    }

    override fun close() {
        runCatching { if (session != null && playUrl.isNotEmpty()) send("TEARDOWN", playUrl, emptyMap()) }
        runCatching { sock?.close() }
    }

    companion object {
        /** ESEE first, then common generic paths. */
        val FALLBACK_PATHS = listOf("/ch0_0.264", "/ch0_1.264", "/live", "/stream1", "/h264")

        /** Reconnect delay after [failures] consecutive failures: 5 s doubling to 5 min, with jitter. */
        fun backoffMs(failures: Int): Long {
            val base = (5_000L shl failures.coerceIn(0, 6)).coerceAtMost(300_000L)
            return base / 2 + (Math.random() * base / 2).toLong()
        }
    }
}
