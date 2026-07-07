package network.hone.app.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter

/**
 * Renders `content` as a QR code on a Compose Canvas (via ZXing's bit matrix).
 * Used for wallet RECEIVE — the user shows this instead of copy-pasting an
 * address ("push button, no copy-paste"). Drawn as vector rects so it scales
 * crisply at any size.
 */
@Composable
fun QrCode(
    content: String,
    modifier: Modifier = Modifier,
    sizeDp: Int = 220,
) {
    val matrix = remember(content) {
        runCatching {
            QRCodeWriter().encode(content, BarcodeFormat.QR_CODE, 96, 96)
        }.getOrNull()
    }

    Canvas(
        modifier
            .size(sizeDp.dp)
            .clip(RoundedCornerShape(16.dp))
            .background(Color.White)
            .padding(12.dp)
            .semantics { contentDescription = "QR code for $content" }
    ) {
        val m = matrix ?: return@Canvas
        val cells = m.width
        val cell = size.width / cells
        for (y in 0 until cells) {
            for (x in 0 until cells) {
                if (m.get(x, y)) {
                    drawRect(
                        color = Color.Black,
                        topLeft = Offset(x * cell, y * cell),
                        size = Size(cell + 0.5f, cell + 0.5f), // +0.5 avoids hairline gaps
                    )
                }
            }
        }
    }
}
