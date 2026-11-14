package network.hone.app.fw

import org.bouncycastle.crypto.generators.Ed25519KeyPairGenerator
import org.bouncycastle.crypto.params.Ed25519KeyGenerationParameters
import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.security.SecureRandom

class FirmwareTest {
    private fun res(name: String): ByteArray = javaClass.classLoader!!.getResourceAsStream("fw/hone-2.2.0/$name")!!.use { it.readBytes() }
    private fun refused(f: () -> Unit) { try { f(); fail("expected refusal") } catch (_: FirmwarePackage.Refused) { } }

    /** The real signed hone-2.2.0 release verifies against the pinned key. */
    @Test fun realRelease220VerifiesWithPinnedKey() {
        val r = FirmwarePackage.verify(res("manifest.json"), res("manifest.json.sig"), appVersionCode = 30)
        assertEquals("2.2.0", r.version)
        assertEquals("/ext/update/f7-update-hone-2.2.0", r.uploadDir)
        assertEquals("/ext/update/f7-update-hone-2.2.0/update.fuf", r.start)
        assertEquals(listOf("firmware.dfu", "radio.bin", "resources.ths", "splash.bin", "update.fuf", "updater.bin"), r.files.map { it.name })
        assertEquals("f7-update-hone-2.2.0/update.fuf", r.manifest!!.url)
        assertFalse(r.files.any { it.url.endsWith(".tgz") })                                      // bundle is not uploaded
    }

    @Test fun realReleaseRefusedWhenTamperedOrUnpinned() {
        val m = res("manifest.json"); val sig = res("manifest.json.sig")
        refused { FirmwarePackage.verify(m, sig, emptyList()) }                                    // fail closed
        val t = m.copyOf(); t[t.indexOf('2'.code.toByte())] = '3'.code.toByte()
        refused { FirmwarePackage.verify(t, sig) }                                                 // one byte changed
        refused { FirmwarePackage.verify(m, sig.copyOf().also { it[0] = (it[0] + 1).toByte() }) }
        refused { FirmwarePackage.verify(m, ByteArray(0)) }
    }

    // Synthetic releases, signed with a throwaway key, for the checks the real one cannot exercise.
    private val kp = Ed25519KeyPairGenerator().apply { init(Ed25519KeyGenerationParameters(SecureRandom())) }.generateKeyPair()
    private val pubHex = (kp.public as Ed25519PublicKeyParameters).encoded.joinToString("") { "%02x".format(it) }
    private fun sign(m: ByteArray) = Ed25519Signer().run { init(true, kp.private as Ed25519PrivateKeyParameters); update(m, 0, m.size); generateSignature() }
    private fun manifest(target: String = "f7", dir: String = "/ext/update/x", start: String = "/ext/update/x/update.fuf",
                         path: String = "x/update.fuf", minApp: Int = 0) =
        JSONObject().put("version", "9.9.9").put("target", target).put("min_app", minApp)
            .put("install", JSONObject().put("rpc_upload_dir", dir).put("upload_from", "x/").put("start", start))
            .put("signature", JSONObject().put("alg", "ed25519"))
            .put("files", JSONArray().put(JSONObject().put("path", path).put("size", 5).put("sha256", "ab".repeat(32))))
            .toString().toByteArray()

    @Test fun refusesUnsafeReleases() {
        fun v(m: ByteArray) = FirmwarePackage.verify(m, sign(m), listOf(pubHex), appVersionCode = 30)
        assertEquals("update.fuf", v(manifest()).manifest!!.name)
        refused { v(manifest(target = "f18")) }
        refused { v(manifest(dir = "/ext/apps", start = "/ext/apps/update.fuf")) }
        refused { v(manifest(start = "/ext/update/y/update.fuf")) }
        refused { v(manifest(path = "x/../../int/update.fuf")) }
        refused { v(manifest(minApp = 99)) }
        refused { FirmwarePackage.verify(manifest(), sign(manifest()), FirmwarePackage.RELEASE_KEYS) }   // wrong key
    }

    @Test fun batteryGate() {
        assertTrue(FirmwarePackage.batteryOk(mapOf("charge.level" to "55", "charge.state" to "discharging")).first)
        assertFalse(FirmwarePackage.batteryOk(mapOf("charge.level" to "12", "charge.state" to "discharging")).first)
        assertTrue(FirmwarePackage.batteryOk(mapOf("charge_level" to "12", "charge_state" to "charging")).first)
        assertFalse(FirmwarePackage.batteryOk(emptyMap()).first)
    }

    @Test fun protobufFraming() {
        val w = Rpc.write(7, "/ext/update/x/update.fuf", byteArrayOf(1, 2, 3), true)
        val (msgs, used) = Pb.split(w + w.copyOf(3))
        assertEquals(1, msgs.size); assertEquals(w.size, used)
        val f = Pb.read(msgs[0])
        assertEquals(7L, f[1]!!.first()); assertEquals(1L, f[3]!!.first())
        val req = Pb.read(f[11]!!.first() as ByteArray)
        assertEquals("/ext/update/x/update.fuf", String(req[1]!!.first() as ByteArray))
        val file = Pb.read(req[2]!!.first() as ByteArray)
        assertTrue((file[4]!!.first() as ByteArray).contentEquals(byteArrayOf(1, 2, 3)))
        val r = Rpc.parse(Pb.W().u(1, 7).msg(33, Pb.W().str(1, "firmware.version").str(2, "2.2.0")).toBytes())
        assertEquals(7L, r.id); assertEquals(0L, r.status); assertEquals(33, r.field)
        assertEquals("firmware.version" to "2.2.0", Rpc.kv(r.body))
        val big = Pb.read(Pb.W().u(1, 300).u(2, 1L shl 40).toBytes())
        assertEquals(300L, big[1]!!.first()); assertEquals(1L shl 40, big[2]!!.first())
    }
}
