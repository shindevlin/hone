//! JNI binding for the Android app (`network.hone.app.sound.Voiceprints`). Feature `android`.
//! Strings in and out are JSON; the handle is an opaque pointer. Nothing here panics into Java.

use crate::{Config, Voiceprinter};
use jni::objects::{JByteArray, JClass, JFloatArray, JString};
use jni::sys::{jboolean, jbyteArray, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

type H = Mutex<Voiceprinter>;

fn h<'a>(p: jlong) -> Option<&'a H> {
    if p == 0 {
        None
    } else {
        Some(unsafe { &*(p as *const H) })
    }
}

fn s(env: &mut JNIEnv, j: &JString) -> Option<String> {
    env.get_string(j).ok().map(|v| v.into())
}

fn out(env: &mut JNIEnv, v: Option<String>) -> jstring {
    match v.and_then(|v| env.new_string(v).ok()) {
        Some(j) => j.into_raw(),
        None => std::ptr::null_mut(),
    }
}

fn cfg(json: Option<String>) -> Config {
    json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeNew(mut env: JNIEnv, _c: JClass, cfg_json: JString, state: JByteArray) -> jlong {
    let c = cfg(s(&mut env, &cfg_json));
    let bytes = if state.is_null() { None } else { env.convert_byte_array(&state).ok() };
    catch_unwind(AssertUnwindSafe(|| {
        let v = match bytes {
            Some(b) if !b.is_empty() => Voiceprinter::load(c.clone(), &b).unwrap_or_else(|_| Voiceprinter::new(c)),
            _ => Voiceprinter::new(c),
        };
        Box::into_raw(Box::new(Mutex::new(v))) as jlong
    }))
    .unwrap_or(0)
}

#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeFree(_env: JNIEnv, _c: JClass, p: jlong) {
    if p != 0 {
        drop(unsafe { Box::from_raw(p as *mut H) });
    }
}

#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeSetLocation(_env: JNIEnv, _c: JClass, p: jlong, lat: f64, lon: f64, tz_min: jint) -> jboolean {
    let Some(h) = h(p) else { return JNI_FALSE };
    let mut v = h.lock().unwrap_or_else(|e| e.into_inner());
    v.set_tz_min(tz_min);
    if v.set_location(lat, lon).is_ok() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

/// Returns the Match as JSON, or null when the detection was ignored.
#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeObserve(
    mut env: JNIEnv,
    _c: JClass,
    p: jlong,
    species: JString,
    emb: JFloatArray,
    t: jlong,
    conf: jfloat,
) -> jstring {
    let Some(h) = h(p) else { return std::ptr::null_mut() };
    let Some(sp) = s(&mut env, &species) else { return std::ptr::null_mut() };
    let n = env.get_array_length(&emb).unwrap_or(0).max(0) as usize;
    let mut buf = vec![0f32; n];
    if env.get_float_array_region(&emb, 0, &mut buf).is_err() {
        return std::ptr::null_mut();
    }
    let r = catch_unwind(AssertUnwindSafe(|| {
        let mut v = h.lock().unwrap_or_else(|e| e.into_inner());
        v.observe(&sp, &buf, t.max(0) as u64, conf).and_then(|m| serde_json::to_string(&m).ok())
    }))
    .ok()
    .flatten();
    out(&mut env, r)
}

/// kind: 0 events (drained), 1 candidates, 2 report (arg = max bytes), 3 status (arg ignored, id in `id`).
#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeQuery(mut env: JNIEnv, _c: JClass, p: jlong, kind: jint, arg: jint, id: JString) -> jstring {
    let Some(h) = h(p) else { return std::ptr::null_mut() };
    let id = if id.is_null() { None } else { s(&mut env, &id) };
    let r = catch_unwind(AssertUnwindSafe(|| {
        let mut v = h.lock().unwrap_or_else(|e| e.into_inner());
        match kind {
            0 => serde_json::to_string(&v.drain_events()).ok(),
            1 => serde_json::to_string(&v.candidates()).ok(),
            2 => Some(v.report(arg.max(0) as usize)),
            3 => id.and_then(|i| v.status(&i)).and_then(|c| {
                let back = v.is_back(&c.bird_id);
                let mut j = serde_json::to_value(&c).ok()?;
                j["is_back"] = serde_json::json!(back);
                Some(j.to_string())
            }),
            _ => None,
        }
    }))
    .ok()
    .flatten();
    out(&mut env, r)
}

/// Returns null on success, else the error text.
#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeName(mut env: JNIEnv, _c: JClass, p: jlong, id: JString, name: JString) -> jstring {
    let Some(h) = h(p) else { return out(&mut env, Some("closed".into())) };
    let (Some(id), Some(name)) = (s(&mut env, &id), s(&mut env, &name)) else { return out(&mut env, Some("bad input".into())) };
    let r = {
        let mut v = h.lock().unwrap_or_else(|e| e.into_inner());
        v.name(&id, &name).err().map(|e| e.to_string())
    };
    out(&mut env, r)
}

/// User link ("this is also Ruben"). Returns the surviving id, or null.
#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeLink(mut env: JNIEnv, _c: JClass, p: jlong, a: JString, b: JString) -> jstring {
    let Some(h) = h(p) else { return std::ptr::null_mut() };
    let (Some(a), Some(b)) = (s(&mut env, &a), s(&mut env, &b)) else { return std::ptr::null_mut() };
    let r = h.lock().unwrap_or_else(|e| e.into_inner()).link(&a, &b).ok();
    out(&mut env, r)
}

#[no_mangle]
pub extern "system" fn Java_network_hone_app_sound_Voiceprints_nativeSave(env: JNIEnv, _c: JClass, p: jlong) -> jbyteArray {
    let Some(h) = h(p) else { return std::ptr::null_mut() };
    let b = h.lock().unwrap_or_else(|e| e.into_inner()).save();
    env.byte_array_from_slice(&b).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut())
}
