package network.hone.app.ui.screens

import android.Manifest
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.TotalCaptureResult
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.OptIn
import androidx.camera.camera2.interop.Camera2Interop
import androidx.camera.camera2.interop.ExperimentalCamera2Interop
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import network.hone.app.ui.theme.HoneTextFaint
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.sqrt

/**
 * In-app bird photo with capture evidence (anti-gaming for rare records).
 * Takes 3 frames ~0.4 s apart while recording focus distance, zoom and the phone's motion,
 * and measures parallax between frames. A flat target (a screen or a print) moves as one plane;
 * a real scene has depth. Evidence travels with the photo; only in-app photos can confirm a
 * rare record. Gallery photos can still be shared, they just don't count as evidence.
 */
@OptIn(ExperimentalCamera2Interop::class)
@Composable
fun BirdCamera(onDone: (Bitmap, JSONObject) -> Unit, onCancel: () -> Unit) {
    val ctx = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()
    var granted by remember { mutableStateOf(ctx.checkSelfPermission(Manifest.permission.CAMERA) == android.content.pm.PackageManager.PERMISSION_GRANTED) }
    val perm = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    LaunchedEffect(Unit) { if (!granted) perm.launch(Manifest.permission.CAMERA) }
    var status by remember { mutableStateOf("Hold the phone on the bird. Move it slightly while capturing.") }
    var busy by remember { mutableStateOf(false) }

    val focus = remember { floatArrayOf(-1f) }           // dioptres from the last frame result
    val capture = remember { ImageCapture.Builder().setCaptureMode(ImageCapture.CAPTURE_MODE_MINIMIZE_LATENCY).build() }
    val previewView = remember { PreviewView(ctx) }
    var zoom by remember { mutableFloatStateOf(1f) }

    if (granted) DisposableEffect(owner) {
        val providerF = ProcessCameraProvider.getInstance(ctx)
        providerF.addListener({
            val provider = providerF.get()
            val pb = Preview.Builder()
            Camera2Interop.Extender(pb).setSessionCaptureCallback(object : CameraCaptureSession.CaptureCallback() {
                override fun onCaptureCompleted(s: CameraCaptureSession, r: CaptureRequest, res: TotalCaptureResult) {
                    res.get(CaptureResult.LENS_FOCUS_DISTANCE)?.let { focus[0] = it }
                }
            })
            val preview = pb.build().also { it.setSurfaceProvider(previewView.surfaceProvider) }
            provider.unbindAll()
            val cam = provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, capture)
            cam.cameraInfo.zoomState.observe(owner) { zoom = it.zoomRatio }
        }, ContextCompat.getMainExecutor(ctx))
        onDispose { runCatching { providerF.get().unbindAll() } }
    }

    Column(Modifier.fillMaxSize().padding(16.dp)) {
        Text("Photo in HONE", style = MaterialTheme.typography.titleLarge)
        Text(status, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        Spacer(Modifier.height(8.dp))
        if (granted) AndroidView({ previewView }, Modifier.fillMaxWidth().weight(1f))
        else Text("Camera permission is needed.", modifier = Modifier.weight(1f))
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = onCancel, enabled = !busy) { Text("Cancel") }
            Button(enabled = granted && !busy, onClick = {
                busy = true; status = "Capturing…"
                scope.launch {
                    val motion = MotionRecorder(ctx).apply { start() }
                    val frames = ArrayList<Bitmap>()
                    val focusAt = ArrayList<Float>()
                    for (i in 0 until 3) {
                        takeFrame(ctx, capture)?.let { frames += it; focusAt += focus[0] }
                        if (i < 2) delay(400)
                    }
                    val m = motion.stop()
                    if (frames.size < 2) { status = "Capture failed, try again."; busy = false; return@launch }
                    val par = parallax(frames.first(), frames.last())
                    val f = focusAt.filter { it >= 0f }.averageOrNull()
                    val ev = JSONObject().put("in_app", true).put("frames", frames.size)
                        .put("focus_dioptres", f ?: JSONObject.NULL)
                        .put("focus_m", f?.let { if (it <= 0.01f) 1000.0 else (1.0 / it) } ?: JSONObject.NULL)
                        .put("zoom", zoom.toDouble())
                        .put("parallax_px", par.first).put("motion_px", par.second)
                        .put("gyro_rms", m.first).put("accel_rms", m.second)
                        .put("t", System.currentTimeMillis())
                    onDone(frames[frames.size / 2], ev)
                    busy = false
                }
            }) { Text("Capture") }
        }
    }
}

private fun List<Float>.averageOrNull(): Double? = if (isEmpty()) null else average()

private suspend fun takeFrame(ctx: Context, capture: ImageCapture): Bitmap? {
    val d = CompletableDeferred<Bitmap?>()
    capture.takePicture(ContextCompat.getMainExecutor(ctx), object : ImageCapture.OnImageCapturedCallback() {
        override fun onCaptureSuccess(image: ImageProxy) {
            val bmp = runCatching {
                val buf = image.planes[0].buffer
                val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
                val b = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
                val rot = image.imageInfo.rotationDegrees
                if (rot != 0) Bitmap.createBitmap(b, 0, 0, b.width, b.height, Matrix().apply { postRotate(rot.toFloat()) }, true) else b
            }.getOrNull()
            image.close(); d.complete(bmp)
        }
        override fun onError(e: ImageCaptureException) { d.complete(null) }
    })
    return d.await()
}

/** Gyro and accelerometer RMS while capturing: a hand-held real scene shows motion. */
private class MotionRecorder(ctx: Context) : SensorEventListener {
    private val sm = ctx.getSystemService(SensorManager::class.java)
    private var g = 0.0; private var gn = 0; private var a = 0.0; private var an = 0
    fun start() {
        sm.getDefaultSensor(Sensor.TYPE_GYROSCOPE)?.let { sm.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME) }
        sm.getDefaultSensor(Sensor.TYPE_LINEAR_ACCELERATION)?.let { sm.registerListener(this, it, SensorManager.SENSOR_DELAY_GAME) }
    }
    fun stop(): Pair<Double, Double> {
        sm.unregisterListener(this)
        return (if (gn > 0) sqrt(g / gn) else 0.0) to (if (an > 0) sqrt(a / an) else 0.0)
    }
    override fun onSensorChanged(e: SensorEvent) {
        val s = e.values[0] * e.values[0] + e.values[1] * e.values[1] + e.values[2] * e.values[2]
        if (e.sensor.type == Sensor.TYPE_GYROSCOPE) { g += s; gn++ } else { a += s; an++ }
    }
    override fun onAccuracyChanged(s: Sensor?, a: Int) {}
}

/**
 * Parallax between two frames: block motion vectors on a 160-px grey image, an affine fit to
 * them, and the residual. A flat target fits one affine motion (low residual); a 3-D scene with
 * hand translation leaves residual parallax. Returns (residual px, mean motion px). Experimental:
 * recorded with the photo, thresholds set once real examples are in.
 */
internal fun parallax(a: Bitmap, b: Bitmap): Pair<Double, Double> {
    val w = 160; val h = (a.height * w / a.width).coerceAtLeast(64)
    val ga = grey(Bitmap.createScaledBitmap(a, w, h, true)); val gb = grey(Bitmap.createScaledBitmap(b, w, h, true))
    val bs = 12; val r = 10
    val pts = ArrayList<DoubleArray>()   // x, y, dx, dy
    var y = r; while (y + bs + r < h) {
        var x = r; while (x + bs + r < w) {
            var best = Double.MAX_VALUE; var bdx = 0; var bdy = 0
            var texture = 0.0
            for (yy in 0 until bs) for (xx in 0 until bs) texture += abs(ga[(y + yy) * w + x + xx] - ga[(y + yy) * w + x + xx + 1])
            if (texture / (bs * bs) > 3.0) {
                for (dy in -r..r) for (dx in -r..r) {
                    var sad = 0.0
                    for (yy in 0 until bs step 2) for (xx in 0 until bs step 2)
                        sad += abs(ga[(y + yy) * w + x + xx] - gb[(y + yy + dy) * w + x + xx + dx])
                    if (sad < best) { best = sad; bdx = dx; bdy = dy }
                }
                pts += doubleArrayOf(x.toDouble(), y.toDouble(), bdx.toDouble(), bdy.toDouble())
            }
            x += bs
        }
        y += bs
    }
    if (pts.size < 6) return 0.0 to 0.0
    // least-squares affine: dx = a0 + a1 x + a2 y ; dy = b0 + b1 x + b2 y
    fun fit(col: Int): DoubleArray {
        val m = Array(3) { DoubleArray(3) }; val v = DoubleArray(3)
        for (p in pts) { val f = doubleArrayOf(1.0, p[0], p[1]); for (i in 0..2) { v[i] += f[i] * p[col]; for (j in 0..2) m[i][j] += f[i] * f[j] } }
        return solve3(m, v)
    }
    val ax = fit(2); val ay = fit(3)
    var res = 0.0; var mot = 0.0
    for (p in pts) {
        val ex = p[2] - (ax[0] + ax[1] * p[0] + ax[2] * p[1]); val ey = p[3] - (ay[0] + ay[1] * p[0] + ay[2] * p[1])
        res += ex * ex + ey * ey; mot += sqrt(p[2] * p[2] + p[3] * p[3])
    }
    return sqrt(res / pts.size) to mot / pts.size
}

private fun grey(b: Bitmap): DoubleArray {
    val px = IntArray(b.width * b.height); b.getPixels(px, 0, b.width, 0, 0, b.width, b.height)
    return DoubleArray(px.size) { val c = px[it]; 0.299 * (c shr 16 and 255) + 0.587 * (c shr 8 and 255) + 0.114 * (c and 255) }
}

private fun solve3(m: Array<DoubleArray>, v: DoubleArray): DoubleArray {
    val a = Array(3) { i -> doubleArrayOf(m[i][0], m[i][1], m[i][2], v[i]) }
    for (c in 0..2) {
        val p = (c..2).maxBy { abs(a[it][c]) }; val t = a[c]; a[c] = a[p]; a[p] = t
        if (abs(a[c][c]) < 1e-9) return DoubleArray(3)
        for (r in 0..2) if (r != c) { val f = a[r][c] / a[c][c]; for (k in c..3) a[r][k] -= f * a[c][k] }
    }
    return DoubleArray(3) { a[it][3] / a[it][it] }
}
