package network.hone.app.station

import java.net.DatagramPacket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.MulticastSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.net.URI
import java.net.URLDecoder
import java.util.UUID
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** A camera found on the LAN (ONVIF WS-Discovery and/or an open RTSP port). */
data class FoundCamera(val host: String, val rtspPort: Int?, val onvif: String?, val name: String?)

/** LAN discovery: ONVIF WS-Discovery probe + RTSP port scan (554, 8554) of the phone's /24. Pure Kotlin. */
object Discovery {
    val RTSP_PORTS = listOf(554, 8554)

    fun probeMessage(id: String = UUID.randomUUID().toString()) =
        """<?xml version="1.0" encoding="UTF-8"?><e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope" """ +
            """xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" """ +
            """xmlns:dn="http://www.onvif.org/ver10/network/wsdl"><e:Header><w:MessageID>uuid:$id</w:MessageID>""" +
            """<w:To e:mustUnderstand="true">urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To>""" +
            """<w:Action e:mustUnderstand="true">http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action></e:Header>""" +
            """<e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types></d:Probe></e:Body></e:Envelope>"""

    /** (http device service URLs, name from the onvif scopes). */
    fun parseProbeMatch(xml: String): Pair<List<String>, String?> {
        val xaddrs = Regex("<(?:\\w+:)?XAddrs>([^<]+)</").find(xml)?.groupValues?.get(1)?.trim()?.split(Regex("\\s+"))
            ?.filter { it.startsWith("http://") || it.startsWith("https://") }.orEmpty()
        val scopes = Regex("<(?:\\w+:)?Scopes[^>]*>([^<]+)</").find(xml)?.groupValues?.get(1).orEmpty()
        val name = Regex("onvif://www\\.onvif\\.org/(?:name|hardware)/(\\S+)").findAll(scopes).map {
            runCatching { URLDecoder.decode(it.groupValues[1], "UTF-8") }.getOrDefault(it.groupValues[1])
        }.firstOrNull()
        return xaddrs to name
    }

    /** WS-Discovery multicast probe; (device service URL, name). Android callers hold a Wi-Fi MulticastLock. */
    fun wsDiscovery(timeoutMs: Int = 3_000): List<Pair<String, String?>> {
        val out = LinkedHashMap<String, String?>()
        runCatching {
            MulticastSocket(null as java.net.SocketAddress?).use { s ->
                s.reuseAddress = true; s.bind(InetSocketAddress(0)); s.soTimeout = 500; s.timeToLive = 2
                val msg = probeMessage().toByteArray()
                val grp = InetAddress.getByName("239.255.255.250")
                repeat(2) { s.send(DatagramPacket(msg, msg.size, grp, 3702)) }
                val end = System.currentTimeMillis() + timeoutMs
                val buf = ByteArray(16384)
                while (System.currentTimeMillis() < end) {
                    val p = DatagramPacket(buf, buf.size)
                    try { s.receive(p) } catch (_: SocketTimeoutException) { continue }
                    val (x, n) = parseProbeMatch(String(p.data, 0, p.length))
                    val pick = x.firstOrNull { runCatching { URI(it).host == p.address.hostAddress }.getOrDefault(false) } ?: x.firstOrNull()
                    if (pick != null) out.putIfAbsent(pick, n)
                }
            }
        }
        return out.toList()
    }

    /** Hosts of the /24 containing [ip], minus [ip] (a wider LAN is still scanned as its /24). */
    fun hostsAround(ip: String): List<String> {
        val p = ip.split(".")
        if (p.size != 4) return emptyList()
        val base = p.take(3).joinToString(".")
        return (1..254).map { "$base.$it" }.filter { it != ip }
    }

    /** host -> first open RTSP port. */
    fun scan(hosts: List<String>, ports: List<Int> = RTSP_PORTS, timeoutMs: Int = 350): Map<String, Int> {
        if (hosts.isEmpty()) return emptyMap()
        val pool = Executors.newFixedThreadPool(48)
        try {
            val jobs = hosts.map { h ->
                pool.submit(Callable {
                    ports.firstOrNull { port ->
                        runCatching { Socket().use { it.connect(InetSocketAddress(h, port), timeoutMs) }; true }.getOrDefault(false)
                    }?.let { h to it }
                })
            }
            return jobs.mapNotNull { runCatching { it.get(15, TimeUnit.SECONDS) }.getOrNull() }.toMap()
        } finally { pool.shutdownNow() }
    }

    /** Both methods merged, cameras with ONVIF first. */
    fun discover(localIp: String?, multicast: Boolean = true, extraHosts: List<String> = emptyList()): List<FoundCamera> {
        val onvif = if (multicast) wsDiscovery() else emptyList()
        val open = scan((localIp?.let { hostsAround(it) }.orEmpty() + extraHosts).distinct())
        val byHost = LinkedHashMap<String, FoundCamera>()
        for ((x, name) in onvif) {
            val h = runCatching { URI(x).host }.getOrNull() ?: continue
            byHost[h] = FoundCamera(h, open[h], x, name)
        }
        for ((h, port) in open) if (h !in byHost) byHost[h] = FoundCamera(h, port, null, null)
        return byHost.values.toList()
    }
}
