//! BLE advertisement classification — identifies tracker type from manufacturer
//! data and service UUIDs without requiring pairing or Apple/Google network access.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TagType {
    AirTag,
    AndroidFmd,
    Tile,
    SamsungSmartTag,
    Chipolo,
    Unknown,
}

impl TagType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AirTag         => "AirTag",
            Self::AndroidFmd     => "AndroidFMD",
            Self::Tile           => "Tile",
            Self::SamsungSmartTag => "Samsung",
            Self::Chipolo        => "Chipolo",
            Self::Unknown        => "Unknown",
        }
    }
}

// Bluetooth SIG company IDs
const APPLE_CO:   u16 = 0x004C;
const GOOGLE_CO:  u16 = 0x00E0;
const SAMSUNG_CO: u16 = 0x0075;

// Well-known 16-bit service UUIDs (short form embedded in standard 128-bit UUID).
// Full form: 0000XXXX-0000-1000-8000-00805F9B34FB
const UUID_AIRTAG:      u16 = 0xFD6F; // Apple Nearby Interaction
const UUID_ANDROID_FMD: u16 = 0xFCE0; // Google Find My Device Network
const UUID_ANDROID_FP:  u16 = 0xFE2C; // Google Fast Pair (FMD overlay)
const UUID_TILE:        u16 = 0xFEED; // Tile Inc.
const UUID_SAMSUNG:     u16 = 0xFD5A; // Samsung SmartTag
const UUID_CHIPOLO_A:   u16 = 0xFE9F; // Chipolo (variant A)
const UUID_CHIPOLO_B:   u16 = 0xFE4F; // Chipolo (variant B)

/// Extract the 16-bit Bluetooth short UUID from a 128-bit UUID.
/// Bluetooth Base UUID: 0000XXXX-0000-1000-8000-00805F9B34FB
/// The short UUID occupies bytes 2-3 (big-endian) of the 16-byte representation.
fn short_uuid(u: &uuid::Uuid) -> u16 {
    let b = u.as_bytes();
    u16::from_be_bytes([b[2], b[3]])
}

/// Classify a BLE advertisement based on manufacturer data and service UUIDs.
/// Returns the most specific tag type that matches, or Unknown.
pub fn classify(
    manufacturer_data: &HashMap<u16, Vec<u8>>,
    services: &[uuid::Uuid],
) -> TagType {
    // ── Manufacturer data checks (most reliable) ──────────────────────────────
    if let Some(data) = manufacturer_data.get(&APPLE_CO) {
        // Apple Find My / AirTag advertisements carry a type byte at data[0].
        // 0x12 = Find My Network (offline findable device, includes AirTag)
        // 0x0F = Nearby (active AirPods/Apple Watch — not a tracker, skip)
        // 0x1E = also used by offline findable devices
        if data.len() >= 2 && matches!(data[0], 0x12 | 0x1E) {
            return TagType::AirTag;
        }
    }

    if manufacturer_data.contains_key(&GOOGLE_CO) {
        return TagType::AndroidFmd;
    }

    if let Some(data) = manufacturer_data.get(&SAMSUNG_CO) {
        // Samsung SmartTag advertisements have a non-trivial payload length.
        if data.len() >= 4 {
            return TagType::SamsungSmartTag;
        }
    }

    // ── Service UUID checks ───────────────────────────────────────────────────
    for svc in services {
        match short_uuid(svc) {
            UUID_AIRTAG                     => return TagType::AirTag,
            UUID_ANDROID_FMD | UUID_ANDROID_FP => return TagType::AndroidFmd,
            UUID_TILE                       => return TagType::Tile,
            UUID_SAMSUNG                    => return TagType::SamsungSmartTag,
            UUID_CHIPOLO_A | UUID_CHIPOLO_B => return TagType::Chipolo,
            _ => {}
        }
    }

    TagType::Unknown
}

/// Returns true only for tags we actively want to track (excludes Unknown).
pub fn is_known_tracker(t: &TagType) -> bool {
    !matches!(t, TagType::Unknown)
}
