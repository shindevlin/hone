package network.hone.app.ui

import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.isUnspecified
import androidx.compose.ui.unit.sp

/**
 * One line that never wraps or cuts a word: the font shrinks until the text fits.
 * Use for tab, nav and chip labels, where translations and large system fonts vary.
 */
@Composable
fun FitText(
    text: String,
    modifier: Modifier = Modifier,
    style: TextStyle = LocalTextStyle.current,
    color: Color = Color.Unspecified,
    minScale: Float = 0.5f,
) {
    val base = if (style.fontSize.isUnspecified) 14.sp else style.fontSize
    var scale by remember(text, base) { mutableFloatStateOf(1f) }
    var ready by remember(text, base) { mutableStateOf(false) }
    Text(
        text,
        modifier.drawWithContent { if (ready) drawContent() },
        color = color,
        style = style.copy(fontSize = base * scale),
        maxLines = 1,
        softWrap = false,
        overflow = TextOverflow.Visible,
        onTextLayout = { r ->
            if (r.didOverflowWidth && scale > minScale) scale = (scale - 0.05f).coerceAtLeast(minScale)
            else ready = true
        },
    )
}
