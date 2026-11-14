package network.hone.app.fw

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Phone -> Flipper firmware install over USB-C (CDC-ACM, preferred when plugged in) or Bluetooth LE,
 * both speaking the Flipper RPC.
 *
 * Never-brick rules: the manifest must verify against a pinned key; every file must match its signed
 * size + sha256 on the phone and its md5 on the Flipper's SD card before the update is triggered; the
 * Flipper must be at 30 % battery or charging and have room; the update only starts when the Flipper's
 * updater accepts the package (SystemUpdateRequest OK). The Flipper flashes from its SD card, so a lost
 * link after that point does not matter; before that point nothing on the Flipper has changed except
 * files in /ext/update/<version>/, and a retry resumes (files already there with the right md5 are skipped).
 */
class FirmwareInstaller(private val ctx: Context) {
    sealed class State {
        data object Idle : State()
        data class Working(val step: Step, val done: Long = 0, val total: Long = 0, val detail: String = "") : State()
        data class Done(val version: String, val via: String) : State()
        data class Failed(val step: Step, val reason: String, val retry: Boolean = true) : State()
    }
    enum class Step { MANIFEST, DOWNLOAD, CONNECT, CHECK, UPLOAD, VERIFY, START, REBOOT, CONFIRM }

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state

    /** Which link would be used now. */
    fun link(): String = if (UsbFlipper.find(ctx) != null) "USB" else if (BleFlipper.bonded(ctx).isNotEmpty()) "Bluetooth" else "none"

    suspend fun fetchRelease(url: String = FirmwarePackage.MANIFEST_URL, appVersionCode: Int): FirmwarePackage.Release =
        withContext(Dispatchers.IO) { FirmwarePackage.verify(get(url).toString(Charsets.UTF_8), appVersionCode = appVersionCode) }

    /** Full install; safe to call again after a failure (resumes). */
    suspend fun install(appVersionCode: Int, url: String = FirmwarePackage.MANIFEST_URL) {
        var step = Step.MANIFEST
        var t: FlipperTransport? = null
        try {
            _state.value = State.Working(step)
            val rel = fetchRelease(url, appVersionCode)

            step = Step.DOWNLOAD
            val files = download(rel, url)

            step = Step.CONNECT; _state.value = State.Working(step)
            t = connect()
            val s = RpcSession(t)
            if (!s.ping()) error("the Flipper did not answer")

            step = Step.CHECK; _state.value = State.Working(step, detail = t.name)
            val (batOk, level) = FirmwarePackage.batteryOk(s.powerInfo())
            if (!batOk) { _state.value = State.Failed(step, "battery ${level ?: "?"}%: charge to 30% or plug in", retry = true); return }
            val free = s.freeSpace("/ext") ?: error("no SD card found")
            if (free < rel.totalBytes + 1_048_576) { _state.value = State.Failed(step, "SD card is full: needs ${rel.totalBytes / 1024} KB free"); return }

            step = Step.UPLOAD
            val dir = "/ext/update/hone-${rel.version}"
            s.mkdir("/ext/update"); s.mkdir(dir)
            var sent = 0L
            for (f in rel.files) {
                val data = files.getValue(f.name).readBytes()
                val path = "$dir/${f.name}"
                val want = FirmwarePackage.md5Hex(data)
                if (s.md5(path) == want) { sent += f.size; _state.value = State.Working(step, sent, rel.totalBytes, f.name); continue }
                val base = sent
                if (!s.writeFile(path, data) { n -> _state.value = State.Working(step, base + n, rel.totalBytes, f.name) }) error("write of ${f.name} failed")
                sent += f.size
            }

            step = Step.VERIFY; _state.value = State.Working(step)
            for (f in rel.files) {
                val want = FirmwarePackage.md5Hex(files.getValue(f.name).readBytes())
                if (s.md5("$dir/${f.name}") != want) error("${f.name} on the Flipper does not match; try again")
            }

            step = Step.START; _state.value = State.Working(step)
            val code = s.update("$dir/update.fuf")
            if (code != 0L) { _state.value = State.Failed(step, "the Flipper refused the package (code $code); nothing was changed", retry = false); return }

            step = Step.REBOOT; _state.value = State.Working(step)
            s.rebootToUpdate(); runCatching { t.close() }; t = null

            step = Step.CONFIRM
            val via = link()
            val deadline = System.currentTimeMillis() + 6 * 60_000
            delay(20_000)
            while (System.currentTimeMillis() < deadline) {
                _state.value = State.Working(step, detail = "waiting for the Flipper to restart")
                val v = runCatching {
                    connect().use { c -> FirmwarePackage.versionOf(RpcSession(c).deviceInfo()) }
                }.getOrNull()
                if (v != null && v.contains(rel.version)) { _state.value = State.Done(rel.version, via); return }
                if (v != null) { _state.value = State.Failed(step, "the Flipper restarted on $v, not ${rel.version}; check its screen"); return }
                delay(10_000)
            }
            _state.value = State.Failed(step, "the Flipper has not come back yet. Leave it on; it finishes on its own. Then reconnect and check its version.", retry = false)
        } catch (e: FirmwarePackage.Refused) {
            _state.value = State.Failed(step, "refused: ${e.message}", retry = false)
        } catch (e: Exception) {
            _state.value = State.Failed(step, e.message ?: e.javaClass.simpleName)
        } finally { runCatching { t?.close() } }
    }

    /** USB if a Flipper is plugged in (asks permission), else the bonded Flipper over Bluetooth. */
    private suspend fun connect(): FlipperTransport {
        UsbFlipper.find(ctx)?.let { d -> if (UsbFlipper.permission(ctx, d)) return UsbFlipper.open(ctx, d) }
        val b = BleFlipper.bonded(ctx).firstOrNull() ?: error("no Flipper: plug in the USB cable, or pair it over Bluetooth first")
        return BleFlipper.open(ctx, b)
    }

    /** Downloads into the app cache, keeping verified files from earlier attempts. */
    private suspend fun download(rel: FirmwarePackage.Release, manifestUrl: String): Map<String, File> = withContext(Dispatchers.IO) {
        val dir = File(ctx.cacheDir, "fw/${rel.version}").apply { mkdirs() }
        var got = 0L
        rel.files.associate { f ->
            val out = File(dir, f.name)
            if (!(out.exists() && FirmwarePackage.matches(f, out.readBytes()))) {
                _state.value = State.Working(Step.DOWNLOAD, got, rel.totalBytes, f.name)
                val data = get(URL(URL(manifestUrl), f.url).toString())
                if (!FirmwarePackage.matches(f, data)) throw FirmwarePackage.Refused("${f.name} does not match the signed hash")
                out.writeBytes(data)
            }
            got += f.size; _state.value = State.Working(Step.DOWNLOAD, got, rel.totalBytes, f.name)
            f.name to out
        }
    }

    private fun get(url: String): ByteArray {
        require(url.startsWith("https://")) { "firmware must come over https" }
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 15_000; c.readTimeout = 60_000
        try {
            if (c.responseCode != 200) error("download failed (${c.responseCode})")
            return c.inputStream.use { it.readBytes() }
        } finally { c.disconnect() }
    }
}
