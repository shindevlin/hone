//! Block header — binary-compatible with Whitepaper 2.2 (180-byte header).
//!
//! Layout (all little-endian):
//!   [0..4]    version                    u32
//!   [4..36]   previous_block_hash        [u8; 32]
//!   [36..68]  merkle_root_transactions   [u8; 32]
//!   [68..100] merkle_root_compute_proofs [u8; 32]
//!   [100..132] state_root               [u8; 32]
//!   [132..140] timestamp                u64 (ms)
//!   [140..144] epoch_number             u32
//!   [144..148] difficulty               u32
//!   [148..180] miner_id                 [u8; 32]

use sha2::{Sha256, Digest};
use serde::{Deserialize, Serialize};

pub const HEADER_SIZE: usize = 180;
pub const HASH_SIZE: usize = 32;
pub const ZERO_HASH: [u8; 32] = [0u8; 32];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u32,
    pub previous_block_hash: [u8; 32],
    pub merkle_root_transactions: [u8; 32],
    pub merkle_root_compute_proofs: [u8; 32],
    pub state_root: [u8; 32],
    pub timestamp: u64,
    pub epoch_number: u32,
    pub difficulty: u32,
    pub miner_id: [u8; 32],
}

impl BlockHeader {
    pub fn new(epoch_number: u32, previous_block_hash: [u8; 32], miner_id: [u8; 32]) -> Self {
        Self {
            version: 1,
            previous_block_hash,
            merkle_root_transactions: ZERO_HASH,
            merkle_root_compute_proofs: ZERO_HASH,
            state_root: ZERO_HASH,
            timestamp: now_ms(),
            epoch_number,
            difficulty: 1,
            miner_id,
        }
    }

    pub fn genesis(timestamp: u64) -> Self {
        Self {
            version: 1,
            previous_block_hash: ZERO_HASH,
            merkle_root_transactions: ZERO_HASH,
            merkle_root_compute_proofs: ZERO_HASH,
            state_root: ZERO_HASH,
            timestamp,
            epoch_number: 0,
            difficulty: 1,
            miner_id: ZERO_HASH,
        }
    }

    pub fn serialize(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.version.to_le_bytes());
        buf[4..36].copy_from_slice(&self.previous_block_hash);
        buf[36..68].copy_from_slice(&self.merkle_root_transactions);
        buf[68..100].copy_from_slice(&self.merkle_root_compute_proofs);
        buf[100..132].copy_from_slice(&self.state_root);
        buf[132..140].copy_from_slice(&self.timestamp.to_le_bytes());
        buf[140..144].copy_from_slice(&self.epoch_number.to_le_bytes());
        buf[144..148].copy_from_slice(&self.difficulty.to_le_bytes());
        buf[148..180].copy_from_slice(&self.miner_id);
        buf
    }

    pub fn deserialize(buf: &[u8]) -> Option<Self> {
        if buf.len() < HEADER_SIZE { return None; }
        Some(Self {
            version:                    u32::from_le_bytes(buf[0..4].try_into().ok()?),
            previous_block_hash:        buf[4..36].try_into().ok()?,
            merkle_root_transactions:   buf[36..68].try_into().ok()?,
            merkle_root_compute_proofs: buf[68..100].try_into().ok()?,
            state_root:                 buf[100..132].try_into().ok()?,
            timestamp:                  u64::from_le_bytes(buf[132..140].try_into().ok()?),
            epoch_number:               u32::from_le_bytes(buf[140..144].try_into().ok()?),
            difficulty:                 u32::from_le_bytes(buf[144..148].try_into().ok()?),
            miner_id:                   buf[148..180].try_into().ok()?,
        })
    }

    pub fn hash(&self) -> [u8; 32] {
        let header = self.serialize();
        Sha256::digest(header).into()
    }

    pub fn hash_hex(&self) -> String {
        hex::encode(self.hash())
    }
}

/// A full block: header + JSON payload (ledger entries, proofs, rewards).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub payload: serde_json::Value,
}

impl Block {
    pub fn to_bytes(&self) -> Vec<u8> {
        let header = self.header.serialize();
        let payload = serde_json::to_vec(&self.payload).unwrap_or_default();
        let len = (payload.len() as u32).to_le_bytes();
        let mut out = Vec::with_capacity(HEADER_SIZE + 4 + payload.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&len);
        out.extend_from_slice(&payload);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < HEADER_SIZE + 4 { return None; }
        let header = BlockHeader::deserialize(&data[..HEADER_SIZE])?;
        let len = u32::from_le_bytes(data[HEADER_SIZE..HEADER_SIZE+4].try_into().ok()?) as usize;
        let payload_end = HEADER_SIZE + 4 + len;
        if data.len() < payload_end { return None; }
        let payload: serde_json::Value = serde_json::from_slice(&data[HEADER_SIZE+4..payload_end]).ok()?;
        Some(Self { header, payload })
    }
}

pub fn merkle_root(hashes: &[String]) -> [u8; 32] {
    if hashes.is_empty() { return ZERO_HASH; }
    let mut level: Vec<[u8; 32]> = hashes.iter().map(|h| {
        if h.len() == 64 {
            hex::decode(h).ok()
                .and_then(|b| b.try_into().ok())
                .unwrap_or(ZERO_HASH)
        } else {
            Sha256::digest(h.as_bytes()).into()
        }
    }).collect();

    while level.len() > 1 {
        let mut next = Vec::with_capacity((level.len() + 1) / 2);
        let mut i = 0;
        while i < level.len() {
            let left = level[i];
            let right = if i + 1 < level.len() { level[i+1] } else { level[i] };
            let mut h = Sha256::new();
            h.update(left);
            h.update(right);
            next.push(h.finalize().into());
            i += 2;
        }
        level = next;
    }
    level[0]
}

pub fn hash_str(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
