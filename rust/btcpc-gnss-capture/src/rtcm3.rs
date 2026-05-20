// RTCM3 frame format: 0xD3 | 6-bit-zero + 10-bit-length | payload | 3-byte CRC-24Q

pub struct Frame {
    pub msg_type:     u16,
    pub payload_bytes: usize,
    pub raw:          Vec<u8>,
}

/// Extract ECEF antenna position from RTCM3 type 1005 (Stationary RTK Reference Station ARP).
/// Returns (x_mm, y_mm, z_mm) in units of 0.0001 m (i.e. divide by 10000 for metres).
/// Returns None if the frame is not type 1005 or is malformed.
pub fn extract_1005_position(frame: &Frame) -> Option<(i64, i64, i64)> {
    if frame.msg_type != 1005 { return None; }
    // Payload starts at byte 3. After 12-bit msg_type, the ECEF fields are:
    // bits 12-23: ref station id (12 bits), then reserved/itrf/gps/glo/gal flags
    // ECEF X: bits 34-68 (35 bits, signed), Y: 70-104, Z: 106-140
    // For simplicity parse the known bit offsets from the spec.
    let p = &frame.raw[3..]; // raw payload (without framing bytes and CRC)
    if p.len() < 19 { return None; }

    // Read as big-endian bits. ECEF X starts at bit 34 of payload (0-indexed from bit 0 = MSB of p[0]).
    let x = read_signed_bits(p, 34, 38);
    let y = read_signed_bits(p, 74, 38);
    let z = read_signed_bits(p, 112, 38);
    Some((x, y, z))
}

fn read_signed_bits(data: &[u8], start_bit: usize, bit_count: usize) -> i64 {
    let mut val: u64 = 0;
    for i in 0..bit_count {
        let bit_pos = start_bit + i;
        let byte_idx = bit_pos / 8;
        let bit_idx = 7 - (bit_pos % 8);
        if byte_idx < data.len() {
            if (data[byte_idx] >> bit_idx) & 1 == 1 {
                val |= 1u64 << (bit_count - 1 - i);
            }
        }
    }
    // Sign-extend
    if val >> (bit_count - 1) == 1 {
        (val as i64) - (1i64 << bit_count)
    } else {
        val as i64
    }
}

pub fn parse_frames(buf: &mut Vec<u8>) -> Vec<Frame> {
    let mut frames = Vec::new();
    loop {
        if buf.is_empty() { break; }
        if buf[0] != 0xD3 { buf.remove(0); continue; }
        if buf.len() < 3 { break; }

        let msg_len = (((buf[1] & 0x03) as usize) << 8) | buf[2] as usize;
        let frame_len = 3 + msg_len + 3;
        if buf.len() < frame_len { break; }

        let raw = buf[..frame_len].to_vec();

        let computed = crc24q(&raw[..3 + msg_len]);
        let stored = ((raw[frame_len - 3] as u32) << 16)
            | ((raw[frame_len - 2] as u32) << 8)
            | raw[frame_len - 1] as u32;

        if computed != stored {
            // Bad CRC — skip this byte and try to resync
            buf.remove(0);
            continue;
        }

        let msg_type = if msg_len >= 2 {
            ((raw[3] as u16) << 4) | (raw[4] as u16 >> 4)
        } else {
            0
        };

        frames.push(Frame { msg_type, payload_bytes: frame_len, raw });
        buf.drain(..frame_len);
    }
    frames
}

fn crc24q(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        crc ^= (byte as u32) << 16;
        for _ in 0..8 {
            crc <<= 1;
            if crc & 0x1000000 != 0 {
                crc ^= 0x1864CFB;
            }
        }
    }
    crc & 0xFFFFFF
}
