package network.hone.app.ui.theme

import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

import android.app.Activity
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
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

private val HoneLightColors = lightColorScheme(
    primary = HoneBrand, onPrimary = Color.White,
    primaryContainer = HoneBrand.copy(alpha = 0.12f), onPrimaryContainer = HoneBrand,
    secondary = HoneProven, onSecondary = Color.White,
    tertiary = HoneSkyLight, onTertiary = Color.White,
    background = HonePaper, onBackground = HoneInk,
    surface = HoneCard, onSurface = HoneInk,
    surfaceVariant = HoneCardHi, onSurfaceVariant = HoneDim,
    surfaceContainer = HoneCard, surfaceContainerHigh = HoneCardHi, surfaceContainerHighest = HoneCardHi,
    outline = HoneLine, outlineVariant = HoneLine,
    secondaryContainer = HoneCardHi, onSecondaryContainer = HoneInk,
    error = HoneErr, onError = Color.White,
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
    // Decide the palette here, above MaterialTheme: a write from a child does not recolour the scheme.
    val ctx = view.context
    val modeVersion by network.hone.app.ui.AppMode.version
    val light = remember(modeVersion) { network.hone.app.ui.AppMode.hone(ctx) && network.hone.app.ui.AppMode.light(ctx) }
    ThemeMode.light = light
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = (if (ThemeMode.light) HonePaper else OrnNight).toArgb()
            window.navigationBarColor = (if (ThemeMode.light) HonePaper else OrnNight).toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = ThemeMode.light
        }
    }
    MaterialTheme(colorScheme = if (light) HoneLightColors else OrnColors, typography = HoneTypography, shapes = OrnShapes, content = content)
}
