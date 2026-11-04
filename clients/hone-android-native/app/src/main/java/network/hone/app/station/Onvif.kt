package network.hone.app.station

import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.security.MessageDigest
import java.security.SecureRandom
import java.text.SimpleDateFormat
import java.util.Base64
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * ONVIF (device + media service) with WS-Security UsernameToken (PasswordDigest).
 * Only what a bird station needs: the RTSP stream URI and a hardware id (MAC, else serial)
 * for the hashed station id. Credentials are never logged. Pure Kotlin (JVM-tested).
 */
class Onvif(private val deviceXaddr: String, private val user: String, private val pass: String, private val timeoutMs: Int = 5_000) {

    data class Stream(val host: String, val port: Int, val path: String)

    fun streamUri(): Stream? {
        val media = runCatching { mediaXaddr() }.getOrNull() ?: guessMedia()
        val profiles = call(media, """<GetProfiles xmlns="http://www.onvif.org/ver10/media/wsdl"/>""")
        for (tok in profileTokens(profiles)) {
            val r = runCatching {
                call(media, """<GetStreamUri xmlns="http://www.onvif.org/ver10/media/wsdl"><StreamSetup>""" +
                    """<Stream xmlns="http://www.onvif.org/ver10/schema">RTP-Unicast</Stream>""" +
                    """<Transport xmlns="http://www.onvif.org/ver10/schema"><Protocol>RTSP</Protocol></Transport>""" +
                    """</StreamSetup><ProfileToken>${esc(tok)}</ProfileToken></GetStreamUri>""")
            }.getOrNull() ?: continue
            parseUri(r)?.let { return it }
        }
        return null
    }

    /** ("mac", aa:bb:..) from the first interface, else ("serial", ...), for hashing only. */
    fun hardwareId(): Pair<String, String>? {
        runCatching { call(deviceXaddr, """<GetNetworkInterfaces xmlns="http://www.onvif.org/ver10/device/wsdl"/>""") }
            .getOrNull()?.let { x -> Regex("<(?:\\w+:)?HwAddress>([0-9A-Fa-f:.-]{11,17})<").find(x)?.let { return "mac" to norm(it.groupValues[1]) } }
        runCatching { call(deviceXaddr, """<GetDeviceInformation xmlns="http://www.onvif.org/ver10/device/wsdl"/>""") }
            .getOrNull()?.let { x -> Regex("<(?:\\w+:)?SerialNumber>([^<]{3,64})<").find(x)?.let { return "serial" to it.groupValues[1].trim() } }
        return null
    }

    /** Model name for display ("ESEE IPC ..."), if the camera tells. */
    fun model(): String? = runCatching { call(deviceXaddr, """<GetDeviceInformation xmlns="http://www.onvif.org/ver10/device/wsdl"/>""") }
        .getOrNull()?.let { x ->
            val mf = Regex("<(?:\\w+:)?Manufacturer>([^<]{1,40})<").find(x)?.groupValues?.get(1)?.trim()
            val md = Regex("<(?:\\w+:)?Model>([^<]{1,40})<").find(x)?.groupValues?.get(1)?.trim()
            listOfNotNull(mf, md).joinToString(" ").ifEmpty { null }
        }

    private fun mediaXaddr(): String {
        val x = call(deviceXaddr, """<GetCapabilities xmlns="http://www.onvif.org/ver10/device/wsdl"><Category>Media</Category></GetCapabilities>""")
        return Regex("<(?:\\w+:)?Media\\b[^>]*>.*?<(?:\\w+:)?XAddr>([^<]+)</", RegexOption.DOT_MATCHES_ALL).find(x)?.groupValues?.get(1)?.trim()
            ?: error("no media service")
    }

    private fun guessMedia(): String = URI(deviceXaddr).let { "${it.scheme}://${it.authority}/onvif/media_service" }

    fun call(url: String, body: String): String {
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"; doOutput = true; connectTimeout = timeoutMs; readTimeout = timeoutMs
            setRequestProperty("Content-Type", "application/soap+xml; charset=utf-8")
        }
        try {
            c.outputStream.use { it.write(envelope(body).toByteArray()) }
            val code = c.responseCode
            val text = (if (code < 400) c.inputStream else c.errorStream)?.bufferedReader()?.use { it.readText() }.orEmpty()
            if (code >= 400) {
                val denied = code == 401 || text.contains("NotAuthorized", true)
                throw RtspException(if (denied) 401 else code, if (denied) "wrong username or password" else "ONVIF error $code")
            }
            return text
        } finally { c.disconnect() }
    }

    fun envelope(body: String, nonce: ByteArray = ByteArray(16).also { SecureRandom().nextBytes(it) }, created: String = now()): String =
        """<?xml version="1.0" encoding="UTF-8"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Header>""" +
            """<Security s:mustUnderstand="1" xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd">""" +
            """<UsernameToken><Username>${esc(user)}</Username>""" +
            """<Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest">${digest(nonce, created, pass)}</Password>""" +
            """<Nonce EncodingType="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary">${Base64.getEncoder().encodeToString(nonce)}</Nonce>""" +
            """<Created xmlns="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">$created</Created>""" +
            """</UsernameToken></Security></s:Header><s:Body>$body</s:Body></s:Envelope>"""

    companion object {
        fun digest(nonce: ByteArray, created: String, pass: String): String =
            Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-1").digest(nonce + created.toByteArray() + pass.toByteArray()))

        fun now(): String = SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US).apply { timeZone = TimeZone.getTimeZone("UTC") }.format(Date())

        fun esc(s: String) = s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;")

        /** Profile tokens, those with an audio encoder first. */
        fun profileTokens(xml: String): List<String> {
            val m = Regex("<(?:\\w+:)?Profiles\\b[^>]*\\btoken=\"([^\"]+)\"[^>]*>(.*?)</(?:\\w+:)?Profiles>", RegexOption.DOT_MATCHES_ALL).findAll(xml).toList()
            return m.sortedBy { if (it.groupValues[2].contains("AudioEncoderConfiguration")) 0 else 1 }.map { it.groupValues[1] }
        }

        fun parseUri(xml: String): Stream? {
            val u = Regex("<(?:\\w+:)?Uri>([^<]+)</").find(xml)?.groupValues?.get(1)?.trim()?.replace("&amp;", "&") ?: return null
            val uri = runCatching { URI(u) }.getOrNull() ?: return null
            if (!uri.scheme.equals("rtsp", true)) return null
            val path = (uri.rawPath ?: "/").ifEmpty { "/" } + (uri.rawQuery?.let { "?$it" } ?: "")
            return Stream(uri.host ?: return null, if (uri.port > 0) uri.port else 554, path)   // userinfo dropped
        }

        fun norm(mac: String) = mac.lowercase().replace(Regex("[^0-9a-f]"), "").chunked(2).joinToString(":")
    }
}
