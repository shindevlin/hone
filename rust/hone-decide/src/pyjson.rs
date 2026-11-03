//! Render JSON exactly like Python's `json.dumps(v, ensure_ascii=False, sort_keys=True)`.
//! Open-Jev renders object contexts this way, so token-level parity needs the same text.

use serde_json::Value;

pub fn dumps(v: &Value) -> String {
    let mut s = String::new();
    write(v, &mut s);
    s
}

fn write(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string())
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string())
            } else {
                out.push_str(&float_repr(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => string(s, out),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write(x, out);
            }
            out.push(']');
        }
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                string(k, out);
                out.push_str(": ");
                write(&m[*k], out);
            }
            out.push('}');
        }
    }
}

fn string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Python `repr(float)`: shortest round-trip digits, fixed notation for
/// decimal exponents in [-4, 16), otherwise `1e-05` / `1.5e+16` style.
pub fn float_repr(x: f64) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{:e}", x); // e.g. "-1.2345e-5"
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let e = exp as usize;
            if digits.len() > e + 1 {
                s.push_str(&digits[..=e]);
                s.push('.');
                s.push_str(&digits[e + 1..]);
            } else {
                s.push_str(&digits);
                s.push_str(&"0".repeat(e + 1 - digits.len()));
                s.push_str(".0");
            }
        } else {
            s.push_str("0.");
            s.push_str(&"0".repeat((-exp - 1) as usize));
            s.push_str(&digits);
        }
    } else {
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(if exp < 0 { '-' } else { '+' });
        s.push_str(&format!("{:02}", exp.abs()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floats_like_python() {
        for (x, want) in [
            (1.0, "1.0"), (0.1, "0.1"), (915.4, "915.4"), (-130.0, "-130.0"), (1e-5, "1e-05"),
            (0.0001, "0.0001"), (1e16, "1e+16"), (123456789012345.6, "123456789012345.6"),
            (2.5e-7, "2.5e-07"), (100.0, "100.0"),
        ] {
            assert_eq!(float_repr(x), want);
        }
    }
    #[test]
    fn objects_sorted_with_python_separators() {
        let v: Value = serde_json::from_str(r#"{"b":[1,2.5,"é\n"],"a":{"z":null,"y":true}}"#).unwrap();
        assert_eq!(dumps(&v), r#"{"a": {"y": true, "z": null}, "b": [1, 2.5, "é\n"]}"#);
    }
}
