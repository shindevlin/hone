package network.hone.app.ui.theme

import androidx.compose.ui.graphics.Color

// HONE palette — dark-first, the chain's signature warm-orange accent.
// Sourced from the existing web app design tokens (--orange #f7931a family)
// so the native app reads as the same product.

val HoneOrange       = Color(0xFFF7931A) // primary accent (mining/earn/active)
val HoneOrangeBright = Color(0xFFFFA733) // hover/pressed highlight
val HoneAmber        = Color(0xFFFFC15E) // secondary warm

val HoneBg           = Color(0xFF0A0E17) // app background (matches StatusBar in old config)
val HoneSurface      = Color(0xFF121826) // cards/panels
val HoneSurfaceHi    = Color(0xFF1B2233) // elevated surface
val HoneOutline      = Color(0xFF2A3346) // borders/dividers

val HoneText         = Color(0xFFE8ECF4) // primary text
val HoneTextDim      = Color(0xFFAEB6C6) // secondary text
val HoneTextFaint    = Color(0xFF6B7488) // tertiary/labels

val HoneGreen        = Color(0xFF3DDC84) // healthy / online / confirmed
val HoneRed          = Color(0xFFFF5A5A) // offline / error / slashed
val HoneBlue         = Color(0xFF5AA9FF) // info / links / clock
