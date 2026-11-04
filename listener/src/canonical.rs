//! Canonical JSON and the signed envelope, byte-compatible with the Android app
//! (telemetry/Canonical.kt) and Python `json.dumps(obj, sort_keys=True, separators=(",", ":"),
//! ensure_ascii=False)`: sorted keys, no whitespace, integers, booleans, strings and null only.

use ed25519_dalek::{Signer, SigningKey};
use serde_json::Value;

pub fn encode(v: &Value) -> String {
    let mut s = String::new();
    write(&mut s, v);
    s
}

fn write(sb: &mut String, v: &Value) {
    match v {
        Value::Null => sb.push_str("null"),
        Value::Bool(b) => sb.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                sb.push_str(&i.to_string())
            } else if let Some(u) = n.as_u64() {
                sb.push_str(&u.to_string())
            } else {
                let d = n.as_f64().unwrap_or(0.0);
                assert!(d == d.round() && d.is_finite(), "canonical JSON has no floats: {d}");
                sb.push_str(&(d as i64).to_string())
            }
        }
        Value::String(x) => string(sb, x),
        Value::Array(a) => {
            sb.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    sb.push(',');
                }
                write(sb, x);
            }
            sb.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            sb.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    sb.push(',');
                }
                string(sb, k);
                sb.push(':');
                write(sb, &m[*k]);
            }
            sb.push('}');
        }
    }
}

fn string(sb: &mut String, s: &str) {
    sb.push('"');
    for c in s.chars() {
        match c {
            '"' => sb.push_str("\\\""),
            '\\' => sb.push_str("\\\\"),
            '\n' => sb.push_str("\\n"),
            '\r' => sb.push_str("\\r"),
            '\t' => sb.push_str("\\t"),
            '\u{8}' => sb.push_str("\\b"),
            '\u{c}' => sb.push_str("\\f"),
            c if (c as u32) < 0x20 => sb.push_str(&format!("\\u{:04x}", c as u32)),
            c => sb.push(c),
        }
    }
    sb.push('"');
}

pub fn pub_hex(k: &SigningKey) -> String {
    hex::encode(k.verifying_key().to_bytes())
}

pub fn sign_hex(k: &SigningKey, msg: &str) -> String {
    hex::encode(k.sign(msg.as_bytes()).to_bytes())
}

/// {"payload": <exact signed string>, "key": key_type, "pub", "sig"} (Canonical.envelope).
pub fn envelope(payload: &str, key_type: &str, k: &SigningKey) -> Value {
    serde_json::json!({"payload": payload, "key": key_type, "pub": pub_hex(k), "sig": sign_hex(k, payload)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorted_compact_like_python() {
        let v = json!({"b": 1, "a": [true, null, "x\"y\n"], "é": "ü"});
        assert_eq!(encode(&v), r#"{"a":[true,null,"x\"y\n"],"b":1,"é":"ü"}"#);
    }

    #[test]
    fn envelope_verifies() {
        use ed25519_dalek::Verifier;
        let k = SigningKey::from_bytes(&[7u8; 32]);
        let e = envelope("{\"a\":1}", "seat", &k);
        let sig = ed25519_dalek::Signature::from_slice(&hex::decode(e["sig"].as_str().unwrap()).unwrap()).unwrap();
        assert!(k.verifying_key().verify(b"{\"a\":1}", &sig).is_ok());
    }
}
