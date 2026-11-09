package network.hone.app.ui.theme

import androidx.compose.ui.graphics.Color

// Ornigo "night canopy" tokens (spec 5 Nov 2026). Dark only for v1.
val OrnNight   = Color(0xFF0B1310) // ground
val OrnCanopy  = Color(0xFF141F1A) // surface
val OrnBough   = Color(0xFF1C2A23) // raised
val OrnTwig    = Color(0xFF2A3A32) // line
val OrnInk     = Color(0xFFF2F6EE) // text
val OrnInkDim  = Color(0xFFB3C1B8) // secondary text
val OrnInkFaint = Color(0xFF8E9E94) // tertiary text (~6:1 on night)
val OrnChorus  = Color(0xFFB8F06A) // primary action, live, Regular here, energy
val OrnGold    = Color(0xFFFFD36B) // shiny, rare, Keeper, streak, "is back"
val OrnSky     = Color(0xFF7CC4FF) // sources, Possible, streak freeze
val OrnDawn    = Color(0xFFFF9A4D) // legendary frame; the HONE link only
val OrnAlarm   = Color(0xFFFF8A7A) // errors, always with an icon and words

// HONE (light) tokens, for HONE-mode screens.
val HonePaper  = Color(0xFFF4F2ED)
val HoneCard   = Color(0xFFFFFFFF)
val HoneLine   = Color(0xFFE2DED6)
val HoneInk    = Color(0xFF16191D)
val HoneDim    = Color(0xFF565C63)
val HoneBrand  = Color(0xFFB4520F)
val HoneProven = Color(0xFF2E6B2A)
val HoneQueued = Color(0xFF8A5A00)

// Names used across the existing screens, now mapped onto the Ornigo tokens.
val HoneOrange: Color get() = if (ThemeMode.light) HoneBrand else OrnChorus
val HoneOrangeBright: Color get() = if (ThemeMode.light) HoneBrand else OrnChorus
val HoneAmber: Color get() = if (ThemeMode.light) HoneQueued else OrnGold
val HoneBg: Color get() = if (ThemeMode.light) HonePaper else OrnNight
val HoneSurface: Color get() = if (ThemeMode.light) HoneCard else OrnCanopy
val HoneSurfaceHi: Color get() = if (ThemeMode.light) HoneCardHi else OrnBough
val HoneOutline: Color get() = if (ThemeMode.light) HoneLine else OrnTwig
val HoneText: Color get() = if (ThemeMode.light) HoneInk else OrnInk
val HoneTextDim: Color get() = if (ThemeMode.light) HoneDim else OrnInkDim
val HoneTextFaint: Color get() = if (ThemeMode.light) HoneFaint else OrnInkFaint
val HoneGreen: Color get() = if (ThemeMode.light) HoneProven else OrnChorus
val HoneRed: Color get() = if (ThemeMode.light) HoneErr else OrnAlarm
val HoneBlue: Color get() = if (ThemeMode.light) HoneSkyLight else OrnSky


// HONE light (HONE mode). Text contrast on white: ink ~16:1, dim ~6.8:1, faint ~5:1, brand ~5:1.
val HoneCardHi   = Color(0xFFEDE9E1)
val HoneFaint    = Color(0xFF6B7177)
val HoneErr      = Color(0xFFB3261E)
val HoneSkyLight = Color(0xFF1F5F95)

/** Which palette the shared screens use: HONE light in HONE mode, Ornigo night otherwise. */
object ThemeMode {
    private val state = androidx.compose.runtime.mutableStateOf(false)
    var light: Boolean
        get() = state.value
        set(v) { state.value = v }
}
