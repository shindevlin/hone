package network.hone.app.ui.theme

import android.app.Activity
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat

// Ornigo is dark only for v1 (night canopy).
private val OrnColors = darkColorScheme(
    primary = OrnChorus,
    onPrimary = OrnNight,
    primaryContainer = OrnChorus.copy(alpha = 0.16f),
    onPrimaryContainer = OrnChorus,
    secondary = OrnGold,
    onSecondary = OrnNight,
    tertiary = OrnSky,
    onTertiary = OrnNight,
    background = OrnNight,
    onBackground = OrnInk,
    surface = OrnCanopy,
    onSurface = OrnInk,
    surfaceVariant = OrnBough,
    onSurfaceVariant = OrnInkDim,
    surfaceContainer = OrnCanopy,
    surfaceContainerHigh = OrnBough,
    surfaceContainerHighest = OrnBough,
    outline = OrnTwig,
    outlineVariant = OrnTwig,
    secondaryContainer = OrnBough,
    onSecondaryContainer = OrnInk,
    error = OrnAlarm,
    onError = OrnNight,
)

private val OrnShapes = Shapes(
    extraSmall = RoundedCornerShape(12.dp),
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(20.dp),
    extraLarge = RoundedCornerShape(28.dp),
)

@Composable
@Suppress("UNUSED_PARAMETER")
fun HoneTheme(darkTheme: Boolean = true, content: @Composable () -> Unit) {
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = OrnNight.toArgb()
            window.navigationBarColor = OrnNight.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = false
        }
    }
    MaterialTheme(colorScheme = OrnColors, typography = HoneTypography, shapes = OrnShapes, content = content)
}
