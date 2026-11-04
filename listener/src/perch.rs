//! Perch v2 (Google, Apache-2.0), the 8-bit LiteRT build packaged by Arm: the same model file
//! and hash as the Android app (sound/Perch.kt). Run through the TensorFlow Lite C API,
//! loaded at runtime (libtensorflowlite_c.so / .dylib / tensorflowlite_c.dll next to the exe).
//! Input: 5 s mono at 32 kHz. Outputs used: softmax over 14,795 classes and the 1,536-float
//! embedding (passed to the voiceprint hook, never sent).

use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;

pub const SAMPLE_RATE: u32 = 32_000;
pub const FRAME: usize = 160_000;
pub const EMB: usize = 1536;
pub const MODEL_FILE: &str = "perch_v2_8bit.tflite";
pub const MODEL_ID: &str = "perch-v2-int8-litert@arm";
pub const MODEL_SHA256: &str = "9ccbfc424bf35efd1d1f4604a3339c2bb9e03b1c7f2ca7b8ccea7f0a026ceb0c";
/// Softmax probability to count a species (SoundSensing.SPECIES_MIN).
pub const SPECIES_MIN: f32 = 0.50;
/// Any human class above this drops the frame (SoundSensing.HUMAN_MAX).
pub const HUMAN_MAX: f32 = 0.05;

#[cfg(target_os = "windows")]
pub const LIB_FILE: &str = "tensorflowlite_c.dll";
#[cfg(target_os = "macos")]
pub const LIB_FILE: &str = "libtensorflowlite_c.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
pub const LIB_FILE: &str = "libtensorflowlite_c.so";

type P = *mut c_void;

struct Api {
    _lib: Library,
    model_delete: unsafe extern "C" fn(P),
    opts_delete: unsafe extern "C" fn(P),
    interp_delete: unsafe extern "C" fn(P),
    input: unsafe extern "C" fn(P, c_int) -> P,
    invoke: unsafe extern "C" fn(P) -> c_int,
    out_count: unsafe extern "C" fn(P) -> c_int,
    output: unsafe extern "C" fn(P, c_int) -> P,
    num_dims: unsafe extern "C" fn(P) -> c_int,
    dim: unsafe extern "C" fn(P, c_int) -> c_int,
    copy_from: unsafe extern "C" fn(P, *const c_void, usize) -> c_int,
    copy_to: unsafe extern "C" fn(P, *mut c_void, usize) -> c_int,
}

pub struct Perch {
    api: Api,
    model: P,
    opts: P,
    interp: P,
    label_out: c_int,
    emb_out: c_int,
    pub names: Vec<String>,
    pub human_idx: Vec<usize>,
    pub is_species: Vec<bool>,
    probs: Vec<f32>,
    pub last_embedding: Vec<f32>,
}

// The interpreter is used from one worker thread at a time.
unsafe impl Send for Perch {}

const HUMAN: &str = r"(?i)speech|speaking|voice|conversation|whisper|singing|shout|scream|laugh|giggle|chuckle|chatter|crowd|crying|babbl|cough|sneeze|^homo sapiens$|human";

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

impl Perch {
    pub fn load(res: &Path, threads: i32) -> Result<Self, String> {
        let model_path = res.join(MODEL_FILE);
        let got = sha256_file(&model_path).map_err(|e| format!("model missing at {}: {e}", model_path.display()))?;
        if got != MODEL_SHA256 {
            return Err("model file hash does not match; reinstall the listener".into());
        }
        let names = load_names(&res.join("class_names.json"))?;
        unsafe {
            let lib = Library::new(res.join(LIB_FILE)).map_err(|e| format!("TFLite runtime: {e}"))?;
            macro_rules! sym {
                ($n:literal) => {{
                    let s: Symbol<*const c_void> = lib.get(concat!($n, "\0").as_bytes()).map_err(|e| format!("{}: {e}", $n))?;
                    std::mem::transmute::<*const c_void, _>(*s)
                }};
            }
            let model_create: unsafe extern "C" fn(*const c_char) -> P = sym!("TfLiteModelCreateFromFile");
            let opts_create: unsafe extern "C" fn() -> P = sym!("TfLiteInterpreterOptionsCreate");
            let set_threads: unsafe extern "C" fn(P, c_int) = sym!("TfLiteInterpreterOptionsSetNumThreads");
            let interp_create: unsafe extern "C" fn(P, P) -> P = sym!("TfLiteInterpreterCreate");
            let alloc: unsafe extern "C" fn(P) -> c_int = sym!("TfLiteInterpreterAllocateTensors");
            let api = Api {
                model_delete: sym!("TfLiteModelDelete"),
                opts_delete: sym!("TfLiteInterpreterOptionsDelete"),
                interp_delete: sym!("TfLiteInterpreterDelete"),
                input: sym!("TfLiteInterpreterGetInputTensor"),
                invoke: sym!("TfLiteInterpreterInvoke"),
                out_count: sym!("TfLiteInterpreterGetOutputTensorCount"),
                output: sym!("TfLiteInterpreterGetOutputTensor"),
                num_dims: sym!("TfLiteTensorNumDims"),
                dim: sym!("TfLiteTensorDim"),
                copy_from: sym!("TfLiteTensorCopyFromBuffer"),
                copy_to: sym!("TfLiteTensorCopyToBuffer"),
                _lib: lib,
            };
            let cpath = CString::new(model_path.to_string_lossy().as_bytes()).unwrap();
            let model = model_create(cpath.as_ptr());
            if model.is_null() {
                return Err("could not read the Perch model".into());
            }
            let opts = opts_create();
            set_threads(opts, threads);
            let interp = interp_create(model, opts);
            if interp.is_null() {
                return Err("could not start the Perch model (unsupported ops?)".into());
            }
            if alloc(interp) != 0 {
                return Err("could not allocate Perch tensors".into());
            }
            let n = names.len() as c_int;
            let (mut label_out, mut emb_out) = (-1, -1);
            for i in 0..(api.out_count)(interp) {
                let t = (api.output)(interp, i);
                let nd = (api.num_dims)(t);
                let last = (api.dim)(t, nd - 1);
                if last == n {
                    label_out = i;
                } else if nd == 2 && last == EMB as c_int {
                    emb_out = i;
                }
            }
            if label_out < 0 || emb_out < 0 {
                return Err("Perch model outputs not found".into());
            }
            let hre = regex::Regex::new(HUMAN).unwrap();
            let human_idx = (0..names.len()).filter(|i| hre.is_match(&names[*i])).collect();
            let is_species = names.iter().map(|s| s.contains(' ') && s != "Homo sapiens").collect();
            Ok(Self { api, model, opts, interp, label_out, emb_out, probs: vec![0.0; names.len()], names, human_idx, is_species, last_embedding: vec![0.0; EMB] })
        }
    }

    /// Class probabilities for one 5 s frame (exactly FRAME samples). The input tensor is
    /// overwritten with zeros afterwards so no audio lingers in the interpreter.
    pub fn classify(&mut self, audio: &[f32]) -> Result<&[f32], String> {
        assert_eq!(audio.len(), FRAME);
        unsafe {
            let inp = (self.api.input)(self.interp, 0);
            if (self.api.copy_from)(inp, audio.as_ptr() as *const c_void, FRAME * 4) != 0 {
                return Err("input copy failed".into());
            }
            let rc = (self.api.invoke)(self.interp);
            let zeros = vec![0f32; FRAME];
            (self.api.copy_from)(inp, zeros.as_ptr() as *const c_void, FRAME * 4);
            if rc != 0 {
                return Err("Perch invoke failed".into());
            }
            let lo = (self.api.output)(self.interp, self.label_out);
            (self.api.copy_to)(lo, self.probs.as_mut_ptr() as *mut c_void, self.probs.len() * 4);
            let eo = (self.api.output)(self.interp, self.emb_out);
            (self.api.copy_to)(eo, self.last_embedding.as_mut_ptr() as *mut c_void, EMB * 4);
        }
        Ok(&self.probs)
    }
}

impl Drop for Perch {
    fn drop(&mut self) {
        unsafe {
            (self.api.interp_delete)(self.interp);
            (self.api.opts_delete)(self.opts);
            (self.api.model_delete)(self.model);
        }
    }
}

/// class_names.json: {"Latin name": 1-based index, ...}.
pub fn load_names(p: &Path) -> Result<Vec<String>, String> {
    let txt = std::fs::read_to_string(p).map_err(|e| format!("class names: {e}"))?;
    let m: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&txt).map_err(|e| format!("class names: {e}"))?;
    let mut names = vec![String::new(); m.len()];
    for (k, v) in m {
        let i = v.as_u64().ok_or("class names: bad index")? as usize;
        if i == 0 || i > names.len() {
            return Err("class names: index out of range".into());
        }
        names[i - 1] = k;
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    #[test]
    fn human_regex_catches_audioset_speech_classes() {
        let re = regex::Regex::new(super::HUMAN).unwrap();
        for n in ["Male_speech_and_man_speaking", "Human_voice", "Conversation", "Whispering", "Child_speech_and_kid_speaking", "Homo sapiens", "Laughter"] {
            assert!(re.is_match(n), "{n}");
        }
        for n in ["Megascops cooperi", "Zenaida asiatica", "Frog", "Rain"] {
            assert!(!re.is_match(n), "{n}");
        }
    }
}
