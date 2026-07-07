# HONE app ProGuard/R8 rules.

# Keep the uniffi-generated bindings + JNA (added Phase 0b) — they use reflection
# / native method names that must not be renamed or stripped.
-keep class uniffi.hone_miner.** { *; }
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { *; }

# Compose handles its own keep rules via the plugin; nothing extra needed here.
