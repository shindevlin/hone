// Git object storage helpers — Phase 2 implementation
// Objects are stored as BlobStore entries in btcpc-fs keyed by SHA-256

use sha2::{Digest, Sha256};

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

pub fn git_object_type(data: &[u8]) -> &str {
    if data.starts_with(b"commit ") {
        "commit"
    } else if data.starts_with(b"tree ") {
        "tree"
    } else if data.starts_with(b"blob ") {
        "blob"
    } else if data.starts_with(b"tag ") {
        "tag"
    } else {
        "unknown"
    }
}
