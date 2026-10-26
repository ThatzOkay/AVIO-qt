//! Tiny protobuf wire-format encoders/decoders.
//!
//! Used by the Video/Audio/Input channels for the handful of small messages sent or read per
//! frame (Ack, Start, InputReport, ...) where round-tripping through a full generated message
//! type isn't worth it.
//!
//! Wire-type values (proto2/proto3 are identical here):
//!   0 = varint           (int32, int64, uint32, uint64, bool, enum)
//!   1 = fixed64          (fixed64, sfixed64, double)
//!   2 = length-delimited (string, bytes, sub-message, packed repeated)
//!   5 = fixed32          (fixed32, sfixed32, float)

/// Encode a base-128 varint. `value`'s bit pattern is used directly, so negative inputs
/// round-trip the way proto varints sign int32/int64 (as the varint of their unsigned 64-bit
/// two's-complement bit pattern).
pub fn encode_varint(value: i64) -> Vec<u8> {
    let mut v = value as u64;
    let mut bytes = Vec::new();
    while v > 0x7f {
        bytes.push(((v & 0x7f) | 0x80) as u8);
        v >>= 7;
    }
    bytes.push((v & 0x7f) as u8);
    bytes
}

/// Emit `<tag, varint>` for a varint-typed field.
pub fn field_varint(field_number: u32, value: i64) -> Vec<u8> {
    let mut out = encode_varint((field_number << 3) as i64);
    out.extend(encode_varint(value));
    out
}

/// Emit `<tag, len, bytes>` for a length-delimited field.
pub fn field_len_delim(field_number: u32, data: &[u8]) -> Vec<u8> {
    let mut out = encode_varint(((field_number << 3) | 2) as i64);
    out.extend(encode_varint(data.len() as i64));
    out.extend_from_slice(data);
    out
}

/// Emit `<tag, 4-byte LE float>` for a fixed32 (float) field.
pub fn field_float(field_number: u32, value: f32) -> Vec<u8> {
    let mut out = encode_varint(((field_number << 3) | 5) as i64);
    out.extend_from_slice(&value.to_le_bytes());
    out
}

/// Decode a varint at `off`, returning `(value, bytes_read)`. Values are kept within the
/// u32/int32-safe range (matches the TS original, which only needs this for
/// session_id/configuration_index-sized fields).
pub fn read_varint(buf: &[u8], off: usize) -> (u32, usize) {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    let mut pos = off;
    while pos < buf.len() {
        let b = buf[pos];
        result |= ((b & 0x7f) as u32) << shift;
        pos += 1;
        if b & 0x80 == 0 {
            return (result, pos - off);
        }
        shift += 7;
        if shift >= 32 {
            // overflow protection: consume the rest of the (out-of-range) varint
            while pos < buf.len() && (buf[pos] & 0x80) != 0 {
                pos += 1;
            }
            pos += 1;
            return (result, pos - off);
        }
    }
    (result, pos - off)
}

pub struct ProtoField {
    pub field: u32,
    pub wire: u8,
    pub bytes: Vec<u8>,
}

/// Generic proto decoder — walks every field in `payload` and returns
/// `(field_number, wire_type, value_bytes)` tuples for the caller to dispatch, stopping at the
/// first group/unknown wire type (matches the TS original, which just returns at that point).
pub fn decode_fields(payload: &[u8]) -> Vec<ProtoField> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < payload.len() {
        let (tag_val, tn) = read_varint(payload, off);
        off += tn;
        let wire = (tag_val & 0x7) as u8;
        let field = tag_val >> 3;
        match wire {
            0 => {
                let (_, vn) = read_varint(payload, off);
                if off + vn > payload.len() {
                    break;
                }
                out.push(ProtoField {
                    field,
                    wire,
                    bytes: payload[off..off + vn].to_vec(),
                });
                off += vn;
            }
            1 => {
                if off + 8 > payload.len() {
                    break;
                }
                out.push(ProtoField {
                    field,
                    wire,
                    bytes: payload[off..off + 8].to_vec(),
                });
                off += 8;
            }
            2 => {
                let (len, ln) = read_varint(payload, off);
                off += ln;
                let len = len as usize;
                if off + len > payload.len() {
                    break;
                }
                out.push(ProtoField {
                    field,
                    wire,
                    bytes: payload[off..off + len].to_vec(),
                });
                off += len;
            }
            5 => {
                if off + 4 > payload.len() {
                    break;
                }
                out.push(ProtoField {
                    field,
                    wire,
                    bytes: payload[off..off + 4].to_vec(),
                });
                off += 4;
            }
            _ => break, // groups / unknown
        }
    }
    out
}

/// Decode a varint-encoded field value (consumes the entire bytes buffer).
pub fn decode_varint_value(bytes: &[u8]) -> u32 {
    read_varint(bytes, 0).0
}

pub struct StartMessage {
    pub session_id: i32,
    pub config_index: i32,
}

/// Decode the `Start { session_id=1, configuration_index=2 }` proto from the payload of an
/// AV_MSG.START_INDICATION. Returns `None` for malformed input (no session_id field).
///
/// Wire format: field-1 tag=0x08 varint, field-2 tag=0x10 varint.
pub fn decode_start(payload: &[u8]) -> Option<StartMessage> {
    let mut off = 0usize;
    let mut session_id: i32 = -1;
    let mut config_index: i32 = -1;
    while off < payload.len() {
        let t = payload[off];
        off += 1;
        match t {
            0x08 => {
                let (v, n) = read_varint(payload, off);
                session_id = v as i32;
                off += n;
            }
            0x10 => {
                let (v, n) = read_varint(payload, off);
                config_index = v as i32;
                off += n;
            }
            _ => {
                let (_, n) = read_varint(payload, off);
                off += n;
            }
        }
    }
    if session_id < 0 {
        None
    } else {
        Some(StartMessage {
            session_id,
            config_index,
        })
    }
}
