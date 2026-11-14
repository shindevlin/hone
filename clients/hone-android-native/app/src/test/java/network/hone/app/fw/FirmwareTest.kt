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
import java.util.Base64

class FirmwareTest {
    private val kp = Ed25519KeyPairGenerator().apply { init(Ed25519KeyGenerationParameters(SecureRandom())) }.generateKeyPair()
    private val pubHex = (kp.public as Ed25519PublicKeyParameters).encoded.joinToString("") { "%02x".format(it) }
    private val fuf = "Filetype: Flipper firmware upgrade configuration".toByteArray()

    private fun body(version: String = "1.4.0-hone", target: Int = 7, minApp: Int = 29, sha: String = FirmwarePackage.sha256Hex(fuf)) =
        JSONObject().put("version", version).put("channel", "hone").put("target", target).put("min_app", minApp)
            .put("files", JSONArray().put(JSONObject().put("name", "update.fuf").put("size", fuf.size).put("sha256", sha).put("url", "f7/update.fuf")))
            .toString()

    private fun envelope(b: String, key: Ed25519PrivateKeyParameters = kp.private as Ed25519PrivateKeyParameters): String {
        val msg = (FirmwarePackage.FORMAT + "\n" + b).toByteArray()
        val sig = Ed25519Signer().run { init(true, key); update(msg, 0, msg.size); generateSignature() }
        return JSONObject().put("format", FirmwarePackage.FORMAT).put("body", b).put("sig", Base64.getEncoder().encodeToString(sig)).toString()
    }

    private fun refused(f: () -> Unit) { try { f(); fail("expected refusal") } catch (_: FirmwarePackage.Refused) { } }

    @Test fun verifiesSignedManifest() {
        val r = FirmwarePackage.verify(envelope(body()), listOf(pubHex), appVersionCode = 29)
        assertEquals("1.4.0-hone", r.version); assertEquals("update.fuf", r.manifest!!.name)
        assertTrue(FirmwarePackage.matches(r.files[0], fuf)); assertFalse(FirmwarePackage.matches(r.files[0], fuf + 1))
    }

    @Test fun refusesUnsignedTamperedWrongKeyAndNoKey() {
        refused { FirmwarePackage.verify(envelope(body()), emptyList()) }                                  // fail closed
        val other = Ed25519KeyPairGenerator().apply { init(Ed25519KeyGenerationParameters(SecureRandom())) }.generateKeyPair()
        refused { FirmwarePackage.verify(envelope(body(), other.private as Ed25519PrivateKeyParameters), listOf(pubHex)) }
        val e = JSONObject(envelope(body())); e.put("body", body(version = "6.6.6"))
        refused { FirmwarePackage.verify(e.toString(), listOf(pubHex)) }                                  // body swapped
        refused { FirmwarePackage.verify(JSONObject(envelope(body())).apply { remove("sig") }.toString(), listOf(pubHex)) }
        refused { FirmwarePackage.verify(envelope(body(target = 18)), listOf(pubHex)) }                    // not f7
        refused { FirmwarePackage.verify(envelope(body(minApp = 99)), listOf(pubHex), appVersionCode = 29) }
        refused { FirmwarePackage.verify(envelope(body(sha = "zz")), listOf(pubHex)) }
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
        assertEquals(1, msgs.size); assertEquals(w.size, used)                                          // partial second frame held back
        val f = Pb.read(msgs[0])
        assertEquals(7L, f[1]!!.first()); assertEquals(1L, f[3]!!.first())
        val req = Pb.read(f[11]!!.first() as ByteArray)
        assertEquals("/ext/update/x/update.fuf", String(req[1]!!.first() as ByteArray))
        val file = Pb.read(req[2]!!.first() as ByteArray)
        assertTrue((file[4]!!.first() as ByteArray).contentEquals(byteArrayOf(1, 2, 3)))
        // A response: id 7, OK, device_info key/value
        val resp = Pb.W().u(1, 7).msg(33, Pb.W().str(1, "firmware.version").str(2, "1.4.0-hone")).toBytes()
        val r = Rpc.parse(resp)
        assertEquals(7L, r.id); assertEquals(0L, r.status); assertEquals(33, r.field)
        assertEquals("firmware.version" to "1.4.0-hone", Rpc.kv(r.body))
        assertEquals("1.4.0-hone", FirmwarePackage.versionOf(mapOf("firmware.version" to "1.4.0-hone")))
    }

    @Test fun varintLargeValues() {
        val b = Pb.W().u(1, 300).u(2, 1L shl 40).toBytes()
        val f = Pb.read(b)
        assertEquals(300L, f[1]!!.first()); assertEquals(1L shl 40, f[2]!!.first())
    }
}
