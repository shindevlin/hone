package network.hone.app.station

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class StationCoreTest {
    private val esee = """v=0
o=- 1 1 IN IP4 192.168.1.20
s=ESEE
a=control:*
m=video 0 RTP/AVP 96
a=rtpmap:96 H264/90000
a=control:track1
m=audio 0 RTP/AVP 8
a=rtpmap:8 PCMA/8000
a=control:track2
"""

    @Test fun sdpPicksAudioOnly() {
        val (ctl, m) = Sdp.parse(esee)
        assertEquals("*", ctl)
        assertEquals(2, m.size)
        val a = Sdp.pickAudio(m)!!
        assertEquals("PCMA", a.encoding); assertEquals(8000, a.clockRate); assertEquals("track2", a.control)
    }

    @Test fun sdpAacConfigAndStaticPt() {
        val sdp = "v=0\r\nm=audio 0 RTP/AVP 97\r\na=rtpmap:97 MPEG4-GENERIC/32000/1\r\n" +
            "a=fmtp:97 streamtype=5; profile-level-id=15; mode=AAC-hbr; config=1288; sizelength=13; indexlength=3; indexdeltalength=3\r\n" +
            "a=control:trackID=1\r\nm=audio 0 RTP/AVP 0\r\n"
        val (_, m) = Sdp.parse(sdp)
        assertEquals("MPEG4-GENERIC", m[0].encoding); assertEquals("1288", m[0].fmtp["config"])
        assertEquals(32000 to 1, Rtp.aacConfig("1288"))
        assertEquals("PCMU", m[1].encoding)
        assertEquals(32000, AudioDecoders.rateOf(m[0]))
    }

    @Test fun noAudioTrackIsNull() { assertNull(Sdp.pickAudio(Sdp.parse("m=video 0 RTP/AVP 96\na=rtpmap:96 H264/90000\n").second)) }

    @Test fun rtpHeaderParsing() {
        val p = byteArrayOf(0x80.toByte(), 0x88.toByte(), 0, 1, 0, 0, 0, 160.toByte(), 1, 2, 3, 4, 0xD5.toByte(), 0x55)
        val (pl, ts, marker) = Rtp.payload(p)!!
        assertEquals(2, pl.size); assertEquals(160L, ts); assertTrue(marker)
        assertNull(Rtp.payload(byteArrayOf(0, 1, 2)))
    }

    @Test fun aacAuHeaders() {
        // 2 AUs of 3 and 2 bytes: AU-headers-length = 32 bits; size(13) index(3)
        val h1 = (3 shl 3); val h2 = (2 shl 3)
        val p = byteArrayOf(0, 32, (h1 shr 8).toByte(), h1.toByte(), (h2 shr 8).toByte(), h2.toByte(), 1, 2, 3, 4, 5)
        val u = Rtp.aacUnits(p)
        assertEquals(2, u.size); assertEquals(listOf<Byte>(1, 2, 3), u[0].toList()); assertEquals(listOf<Byte>(4, 5), u[1].toList())
    }

    @Test fun g711Decoding() {
        assertEquals(8, G711.alaw(0xD5)); assertEquals(-8, G711.alaw(0x55))
        assertEquals(0, G711.ulaw(0xFF)); assertEquals(-32124, G711.ulaw(0x00))
        assertEquals(32124, G711.ulaw(0x80))
        assertEquals(2, G711.decode("PCMA", byteArrayOf(1, 2)).size)
    }

    @Test fun resamplerRatioAndContinuity() {
        val r = LinearResampler(8000, 32000)
        val a = r.process(FloatArray(800) { 0.5f }); val b = r.process(FloatArray(800) { 0.5f })
        assertEquals(6400.0, (a.size + b.size).toDouble(), 2.0)
        assertTrue(b.all { kotlin.math.abs(it - 0.5f) < 1e-4 })
        val d = LinearResampler(48000, 32000).process(FloatArray(4800))
        assertTrue(d.size in 3199..3201)
    }

    @Test fun tiers() {
        assertEquals("A", StationQuality.tier(48000)); assertEquals("A", StationQuality.tier(32000))
        assertEquals("B", StationQuality.tier(16000)); assertEquals("B", StationQuality.tier(24000))
        assertEquals("C", StationQuality.tier(8000)); assertEquals("C", StationQuality.tier(11025))
        val nf = StationQuality.noiseFloorDb(FloatArray(8000) { 0.01f }, 8000)!!
        assertEquals(-40.0, nf, 0.2)
    }

    @Test fun digestAuthResponseRfc2617() {
        // RFC 2617 example values (no qop): known MD5 composition.
        val a = RtspAuth("Mufasa", "Circle Of Life")
        assertTrue(a.accept(listOf("Digest realm=\"testrealm@host.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\"")))
        val h = a.header("GET", "/dir/index.html")!!
        val ha1 = RtspAuth.md5("Mufasa:testrealm@host.com:Circle Of Life"); val ha2 = RtspAuth.md5("GET:/dir/index.html")
        assertTrue(h.contains("response=\"${RtspAuth.md5("$ha1:dcd98b7102dd2f0e8b11d0f600bfb0c093:$ha2")}\""))
        // the same challenge again means the password is wrong: no retry loop
        assertFalse(a.accept(listOf("Digest realm=\"testrealm@host.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\"")))
        val b = RtspAuth("u", "p"); assertTrue(b.accept(listOf("Basic realm=\"cam\"")))
        assertEquals("Basic dTpw", b.header("DESCRIBE", "rtsp://x/"))
    }

    @Test fun onvifDigestAndParsing() {
        // WS-Security: Base64(SHA1(nonce + created + password))
        val d = Onvif.digest(ByteArray(16) { it.toByte() }, "2026-11-04T12:00:00Z", "pw")
        val exp = java.util.Base64.getEncoder().encodeToString(java.security.MessageDigest.getInstance("SHA-1")
            .digest(ByteArray(16) { it.toByte() } + "2026-11-04T12:00:00Z".toByteArray() + "pw".toByteArray()))
        assertEquals(exp, d)
        val prof = """<trt:Profiles token="main"><tt:VideoEncoderConfiguration/></trt:Profiles><trt:Profiles token="sub"><tt:AudioEncoderConfiguration/></trt:Profiles>"""
        assertEquals(listOf("sub", "main"), Onvif.profileTokens(prof))
        val s = Onvif.parseUri("<tt:Uri>rtsp://admin:pw@192.168.1.20:554/ch0_1.264?a=1&amp;b=2</tt:Uri>")!!
        assertEquals("192.168.1.20", s.host); assertEquals(554, s.port); assertEquals("/ch0_1.264?a=1&b=2", s.path)
        assertEquals("aa:bb:cc:00:11:22", Onvif.norm("AA-BB-CC-00-11-22"))
    }

    @Test fun discoveryParsing() {
        val xml = """<d:ProbeMatch><d:Scopes>onvif://www.onvif.org/type/video onvif://www.onvif.org/name/ESEE%20IPC</d:Scopes>
            <d:XAddrs>http://192.168.1.20:8899/onvif/device_service http://[fe80::1]/onvif</d:XAddrs></d:ProbeMatch>"""
        val (x, n) = Discovery.parseProbeMatch(xml)
        assertEquals("http://192.168.1.20:8899/onvif/device_service", x[0]); assertEquals("ESEE IPC", n)
        val hosts = Discovery.hostsAround("192.168.1.5")
        assertEquals(253, hosts.size); assertFalse("192.168.1.5" in hosts)
        assertTrue(Discovery.probeMessage("x").contains("NetworkVideoTransmitter"))
    }

    @Test fun stationIdIsHashedAndStable() {
        val a = StationSetup.stationId("mac", "AA:BB:CC:00:11:22", "ff".repeat(32))
        assertEquals(a, StationSetup.stationId("mac", "aa:bb:cc:00:11:22", "ff".repeat(32)))
        assertEquals(16, a.length); assertFalse(a.contains("aa:bb"))
        assertTrue(RtspAudioClient.backoffMs(0) in 2_500..5_000)
        assertTrue(RtspAudioClient.backoffMs(20) <= 300_000)
    }

    /**
     * End to end against a real RTSP server (mediamtx + ffmpeg on grouchly). Runs only when
     * /tmp/hone_rtsp_test.properties exists: host, port, path, user, pass, expect_codec.
     */
    @Test fun liveRtspServer() {
        val f = File("/tmp/hone_rtsp_test.properties"); if (!f.exists()) return
        val p = java.util.Properties().apply { f.inputStream().use { load(it) } }
        val cam = FoundCamera(p.getProperty("host"), p.getProperty("port").toInt(), null, "test")
        // wrong password is reported as such (401), not as "unreachable"
        val bad = runCatching { StationSetup.connect(cam, p.getProperty("user"), "wrong", seconds = 2) }.exceptionOrNull()
        assertEquals(401, (bad as RtspException).code)
        // ONVIF absent: path fallback must find the stream (ESEE path first)
        val r = StationSetup.connect(cam, p.getProperty("user"), p.getProperty("pass"), seconds = 4)
        assertEquals(p.getProperty("path"), r.path); assertEquals(p.getProperty("expect_codec"), r.codec)
        assertEquals(StationQuality.tier(r.rate), r.tier); assertEquals(1, r.videoTracksSkipped)
        assertNotNull(r.noiseDb)
        File("/tmp/hone_rtsp_test.result").writeText("codec=${r.codec} rate=${r.rate} tier=${r.tier} kbps=${r.kbps} noise=${r.noiseDb} path=${r.path} video_skipped=${r.videoTracksSkipped}\n")
    }
}
