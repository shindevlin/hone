package network.hone.app.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import network.hone.app.ui.theme.*

/** Shared pieces of the HONE-mode screens (spec: HONE section, paper theme, plain words). */

@Composable
fun HoneBox(modifier: Modifier = Modifier, border: Color = HoneOutline, onClick: (() -> Unit)? = null,
            content: @Composable ColumnScope.() -> Unit) {
    val shape = RoundedCornerShape(18.dp)
    Column(
        modifier.fillMaxWidth().clip(shape).background(HoneSurface).border(1.dp, border, shape)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp), content = content,
    )
}

@Composable
fun HoneTitle(text: String, modifier: Modifier = Modifier) =
    Text(text, style = MaterialTheme.typography.headlineMedium.copy(fontSize = 28.sp, lineHeight = 32.sp), color = HoneText, modifier = modifier)

@Composable
fun Dim(text: String, modifier: Modifier = Modifier, color: Color = HoneTextDim) =
    Text(text, style = MaterialTheme.typography.bodySmall.copy(fontSize = 13.sp, lineHeight = 19.sp), color = color, modifier = modifier)

@Composable
fun Mono(text: String, size: Int = 14, color: Color = HoneText, bold: Boolean = true) =
    Text(text, fontFamily = HoneMono, fontSize = size.sp, fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal, color = color)

/** Technical name under a plain label, for experts only. */
@Composable
fun Tech(text: String) = Text(text, fontFamily = HoneMono, fontSize = 11.sp, color = HoneTextFaint)

/** Segmented tabs ("Send | Back someone"). */
@Composable
fun Segmented(options: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(HoneSurfaceHi).padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        options.forEachIndexed { i, label ->
            val on = i == selected
            Box(Modifier.weight(1f).height(44.dp).clip(RoundedCornerShape(10.dp))
                .background(if (on) HoneSurface else Color.Transparent)
                .semantics { this.selected = on }
                .clickable(role = Role.Tab) { onSelect(i) }, contentAlignment = Alignment.Center) {
                Text(label, fontWeight = FontWeight.Bold, fontSize = 13.sp, color = if (on) HoneText else HoneTextDim, maxLines = 1)
            }
        }
    }
}

/** Waiting-to-be-checked is drawn striped, so it reads as "not yet" without colour alone. */
@Composable
fun SplitBar(paid: Float, waiting: Float, height: Dp = 10.dp, label: String) {
    val total = (paid + waiting).takeIf { it > 0f } ?: 1f
    val stripeA = HoneAmber; val stripeB = HoneAmber.copy(alpha = 0.28f)
    Canvas(Modifier.fillMaxWidth().height(height).clip(RoundedCornerShape(height / 2)).semantics { contentDescription = label }) {
        val pw = size.width * paid / total
        drawRect(HoneOutline)
        drawRect(HoneGreen, size = size.copy(width = pw))
        if (waiting > 0f) {
            val w0 = pw; val w1 = pw + size.width * waiting / total
            drawRect(stripeB, topLeft = Offset(w0, 0f), size = size.copy(width = w1 - w0))
            val step = 8.dp.toPx(); var x = w0 - size.height
            clipRect(left = w0, right = w1) {
                while (x < w1) { drawLine(stripeA, Offset(x, size.height), Offset(x + size.height, 0f), strokeWidth = 3.dp.toPx()); x += step }
            }
        }
    }
}

/** A plain progress bar ("34 of 100"). */
@Composable
fun ProgressLine(fraction: Float, color: Color = HoneGreen, label: String) {
    Box(Modifier.fillMaxWidth().height(8.dp).clip(RoundedCornerShape(4.dp)).background(HoneSurfaceHi).semantics { contentDescription = label }) {
        Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).fillMaxHeight().background(color))
    }
}

@Composable
fun Dot(color: Color, size: Dp = 10.dp, hollow: Boolean = false) =
    Box(Modifier.size(size).clip(CircleShape).then(if (hollow) Modifier.border(3.dp, color, CircleShape) else Modifier.background(color)))

@Composable
fun InkButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) =
    Button(onClick, modifier.fillMaxWidth().height(52.dp), enabled = enabled, shape = RoundedCornerShape(26.dp),
        colors = ButtonDefaults.buttonColors(containerColor = HoneText, contentColor = HoneBg)) { Text(text, fontWeight = FontWeight.Bold) }

@Composable
fun LineButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) =
    OutlinedButton(onClick, modifier.height(44.dp), enabled = enabled, shape = RoundedCornerShape(22.dp)) { Text(text, fontWeight = FontWeight.Bold, color = HoneText) }

/** Expert row: plain label, technical name under it, value at the end, details on tap. */
@Composable
fun ExpertRow(label: String, tech: String, value: String, valueColor: Color = HoneTextDim, detail: String? = null,
              extra: (@Composable ColumnScope.() -> Unit)? = null) {
    var open by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().clickable(enabled = detail != null || extra != null) { open = !open }.padding(horizontal = 14.dp, vertical = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(label, fontWeight = FontWeight.Bold, fontSize = 15.sp, color = HoneText)
                Tech(tech)
            }
            Text(value, fontSize = 13.sp, color = valueColor)
        }
        if (open) {
            Spacer(Modifier.height(6.dp))
            detail?.let { Dim(it) }
            extra?.invoke(this)
        }
    }
}

/** Something that exists in the app but is still switched off on the chain. */
@Composable
fun ComingSoon(title: String, body: String, state: ChainGate) {
    HoneBox {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(title, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f), color = HoneText)
            Text(when (state) { ChainGate.Live -> "ON"; ChainGate.Off -> "COMING SOON"; ChainGate.Unknown -> "CHECKING" },
                fontSize = 12.sp, fontWeight = FontWeight.Bold, color = if (state == ChainGate.Live) HoneGreen else HoneAmber)
        }
        Dim(body)
        Dim(when (state) {
            ChainGate.Off -> "The network has not switched this on yet. This screen turns on by itself when it does."
            ChainGate.Unknown -> "Asking the network whether this is on…"
            ChainGate.Live -> "On."
        }, color = HoneTextFaint)
    }
}

enum class ChainGate { Unknown, Off, Live }
