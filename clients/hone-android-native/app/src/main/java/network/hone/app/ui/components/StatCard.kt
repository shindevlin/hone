package network.hone.app.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import network.hone.app.ui.theme.HoneMono
import network.hone.app.ui.theme.HoneTextFaint

/**
 * A single labelled stat tile (epoch, peers, balance…). Monospace value so
 * ticking numbers don't jitter. Accessible: the whole card is one semantics
 * node reading "<label>: <value>".
 */
@Composable
fun StatCard(
    label: String,
    value: String,
    icon: ImageVector,
    modifier: Modifier = Modifier,
    valueMono: Boolean = true,
    accent: Boolean = false,
) {
    val surface = MaterialTheme.colorScheme.surface
    val outline = MaterialTheme.colorScheme.outline
    val valueColor by animateColorAsState(
        if (accent) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
        label = "statValueColor",
    )

    Column(
        modifier = modifier
            .background(surface, RoundedCornerShape(16.dp))
            .border(1.dp, outline, RoundedCornerShape(16.dp))
            .padding(16.dp)
            .semantics { contentDescription = "$label: $value" },
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
        ) {
            Icon(icon, contentDescription = null, tint = HoneTextFaint, modifier = Modifier.size(16.dp))
            Text(label, style = MaterialTheme.typography.labelSmall, color = HoneTextFaint)
        }
        Text(
            value,
            style = MaterialTheme.typography.titleLarge.copy(
                fontFamily = if (valueMono) HoneMono else FontFamily.Default,
            ),
            color = valueColor,
        )
    }
}
