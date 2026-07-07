package network.hone.app.ui.theme

import android.app.Activity
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

// HONE is a dark-first product; the dark scheme is the real design. A light
// scheme is provided for completeness/accessibility (some users force light).

private val DarkColors = darkColorScheme(
    primary = HoneOrange,
    onPrimary = Color.Black,
    primaryContainer = HoneOrange.copy(alpha = 0.16f),
    onPrimaryContainer = HoneOrangeBright,
    secondary = HoneAmber,
    onSecondary = Color.Black,
    background = HoneBg,
    onBackground = HoneText,
    surface = HoneSurface,
    onSurface = HoneText,
    surfaceVariant = HoneSurfaceHi,
    onSurfaceVariant = HoneTextDim,
    outline = HoneOutline,
    error = HoneRed,
    onError = Color.Black,
)

private val LightColors = lightColorScheme(
    primary = HoneOrange,
    onPrimary = Color.White,
    secondary = HoneAmber,
    background = Color(0xFFFAFAFB),
    surface = Color.White,
    error = HoneRed,
)

@Composable
fun HoneTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val colors = if (darkTheme) DarkColors else LightColors

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as Activity).window
            window.statusBarColor = HoneBg.toArgb()
            window.navigationBarColor = HoneBg.toArgb()
            WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars = !darkTheme
        }
    }

    MaterialTheme(
        colorScheme = colors,
        typography = HoneTypography,
        content = content,
    )
}
