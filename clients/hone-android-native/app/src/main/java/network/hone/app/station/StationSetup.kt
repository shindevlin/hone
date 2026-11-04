package network.hone.app.station

import java.security.MessageDigest

/** Decodes one RTP payload of the audio track to mono floats at [rate]. */
interface AudioDecoder : AutoCloseable {
    val rate: Int
    fun decode(payload: ByteArray): FloatArray
    override fun close() {}
}

/** G.711 A-law / µ-law and L16: no codec needed. */
class PcmDecoder(private val media: SdpMedia) : AudioDecoder {
    override val rate = media.clockRate
    override fun decode(payload: ByteArray) = G711.decode(media.encoding, payload, media.channels)
}

object AudioDecoders {
    /** Sample rate the track carries (AAC: from its AudioSpecificConfig). */
    fun rateOf(m: SdpMedia): Int =
        if (m.encoding == "MPEG4-GENERIC") m.fmtp["config"]?.let { Rtp.aacConfig(it)?.first } ?: m.clockRate else m.clockRate

    /** [aac] builds the platform AAC decoder (MediaCodec on Android); null = AAC not decodable here. */
    fun forMedia(m: SdpMedia, aac: ((SdpMedia) -> AudioDecoder)?): AudioDecoder? = when (m.encoding) {
        "PCMA", "PCMU", "L16" -> PcmDecoder(m)
        "MPEG4-GENERIC" -> aac?.invoke(m)
        else -> null
    }
}

data class ProbeResult(
    val host: String, val port: Int, val path: String, val codec: String, val rate: Int, val tier: String,
    val kbps: Int, val noiseDb: Double?, val seconds: Int, val idSource: String, val idValue: String,
    val model: String?, val videoTracksSkipped: Int,
)

/**
 * Finds the camera's stream (ONVIF GetStreamUri, then common paths), opens its audio track
 * and listens for a few seconds to rate it. Throws RtspException with code 401 (bad login),
 * 415 (no usable audio) or 0 (unreachable). Pure Kotlin; the Android side passes the AAC
 * decoder and an ARP lookup.
 */
object StationSetup {
    fun candidates(cam: FoundCamera, onvifStream: Onvif.Stream?): List<Pair<Int, String>> {
        val out = LinkedHashSet<Pair<Int, String>>()
        onvifStream?.let { out += it.port to it.path }
        val ports = listOfNotNull(cam.rtspPort, onvifStream?.port, 554, 8554).distinct()
        for (port in ports) for (p in RtspAudioClient.FALLBACK_PATHS) out += port to p
        return out.toList()
    }

    fun connect(
        cam: FoundCamera, user: String, pass: String, seconds: Int = 6,
        aac: ((SdpMedia) -> AudioDecoder)? = null, arpMac: (String) -> String? = { null },
    ): ProbeResult {
        var onvifStream: Onvif.Stream? = null
        var hw: Pair<String, String>? = null
        var model: String? = cam.name
        cam.onvif?.let { x ->
            val o = Onvif(x, user, pass)
            try { onvifStream = o.streamUri() } catch (e: RtspException) { if (e.code == 401) throw e }
            catch (_: Exception) {}
            hw = runCatching { o.hardwareId() }.getOrNull()
            runCatching { o.model() }.getOrNull()?.let { model = it }
        }
        var noAudio: RtspException? = null
        var reached = false
        for ((port, path) in candidates(cam, onvifStream)) {
            val c = RtspAudioClient(cam.host, port, path, user.ifEmpty { null }, pass, timeoutMs = 5_000)
            try {
                val m = c.open()
                reached = true
                val dec = AudioDecoders.forMedia(m, aac) ?: throw RtspException(415, "audio codec not supported here: ${m.encoding}")
                val samples = ArrayList<FloatArray>()
                val t0 = System.currentTimeMillis()
                dec.use {
                    c.stream({ System.currentTimeMillis() - t0 > seconds * 1000L }) { payload, _, _ ->
                        val pcm = if (m.encoding == "MPEG4-GENERIC")
                            Rtp.aacUnits(payload, m.fmtp["sizelength"]?.toIntOrNull() ?: 13, m.fmtp["indexlength"]?.toIntOrNull() ?: 3)
                                .map { u -> dec.decode(u) }.fold(FloatArray(0)) { a, b -> a + b }
                        else dec.decode(payload)
                        if (pcm.isNotEmpty()) samples += pcm
                    }
                }
                val secs = ((System.currentTimeMillis() - t0) / 1000.0).coerceAtLeast(1.0)
                val all = FloatArray(samples.sumOf { it.size }).also { a -> var o = 0; samples.forEach { s -> s.copyInto(a, o); o += s.size } }
                val rate = AudioDecoders.rateOf(m)
                if (all.isEmpty()) throw RtspException(415, "no sound arrived from the camera")
                val id = hw ?: arpMac(cam.host)?.let { "mac" to it } ?: ("host" to cam.host)
                return ProbeResult(cam.host, port, path, m.encoding, rate, StationQuality.tier(rate),
                    (c.bytesIn * 8 / secs / 1000).toInt(), StationQuality.noiseFloorDb(all, dec.rate), secs.toInt(),
                    id.first, id.second, model, c.videoTracksSkipped)
            } catch (e: RtspException) {
                if (e.code == 401) throw e
                if (e.code == 415) { noAudio = e; reached = true }
            } catch (_: java.io.IOException) {
            } finally { c.close() }
        }
        throw noAudio ?: RtspException(if (reached) 404 else 0, "camera stream not found")
    }

    /** Stable station id: hash of the camera's hardware id and this device's public key. Never the raw MAC. */
    fun stationId(idSource: String, idValue: String, seatPubHex: String): String =
        MessageDigest.getInstance("SHA-256").digest("hone-station|$idSource|${idValue.lowercase()}|$seatPubHex".toByteArray())
            .joinToString("") { "%02x".format(it) }.take(16)
}
