use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jstring, jboolean};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod flipper_rx;
mod miner;
mod proof;

static RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
static MINER_STATE: OnceCell<Arc<MinerState>> = OnceCell::new();
// Shared HTTP client for the Flipper-frame ingest path (separate from
// miner::run_miner's own client — that one is scoped to its async task).
static FLIPPER_HTTP: OnceCell<reqwest::Client> = OnceCell::new();

pub struct MinerState {
    pub running: AtomicBool,
    pub status: Mutex<String>,
    /// Populated at nativeStart time so nativeIngestFrame (a separate JNI
    /// entry point, called from the Flipper BLE path) can submit signed
    /// readings without its own start/stop lifecycle.
    pub api_base: Mutex<String>,
    pub account: Mutex<String>,
    pub posting_key: Mutex<String>,
    /// Chain id used in the SENSOR_READING canonical signing message.
    /// Defaults to the testnet id; overridden if the app ever passes one in.
    pub chain_id: Mutex<String>,
}

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("hone-miner")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

fn state() -> &'static Arc<MinerState> {
    MINER_STATE.get_or_init(|| {
        Arc::new(MinerState {
            running: AtomicBool::new(false),
            status: Mutex::new("Stopped".to_string()),
            api_base: Mutex::new(String::new()),
            account: Mutex::new(String::new()),
            posting_key: Mutex::new(String::new()),
            chain_id: Mutex::new("hone-satoshi".to_string()),
        })
    })
}

fn flipper_http_client() -> &'static reqwest::Client {
    FLIPPER_HTTP.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("flipper http client")
    })
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    j_account: JString,
    j_jwt: JString,
    j_api_base: JString,
    j_model_id: JString,
    j_model_dir: JString,
    j_posting_key: JString,
) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("hone-miner"),
    );

    let account:     String = env.get_string(&j_account).map(|s| s.into()).unwrap_or_default();
    let jwt:         String = env.get_string(&j_jwt).map(|s| s.into()).unwrap_or_default();
    let api_base:    String = env.get_string(&j_api_base).map(|s| s.into()).unwrap_or_default();
    let model_id:    String = env.get_string(&j_model_id).map(|s| s.into())
                                .unwrap_or_else(|_| "hone-phone-v1".to_string());
    let model_dir:   String = env.get_string(&j_model_dir).map(|s| s.into()).unwrap_or_default();
    let posting_key: String = env.get_string(&j_posting_key).map(|s| s.into()).unwrap_or_default();

    let st = state().clone();
    if st.running.swap(true, Ordering::SeqCst) {
        return;
    }
    *st.status.lock() = "Starting…".to_string();

    // Stash for nativeIngestFrame (Flipper BLE path), which has no start/stop
    // lifecycle of its own and runs independently of the miner loop below.
    *st.api_base.lock() = api_base.clone();
    *st.account.lock() = account.clone();
    *st.posting_key.lock() = posting_key.clone();

    runtime().spawn(async move {
        *state().status.lock() = format!("Loading {model_id}…");
        match miner::run_miner(account, jwt, api_base, model_id.clone(), model_dir, posting_key, state()).await {
            Ok(_) => { *state().status.lock() = "Stopped".to_string(); }
            Err(e) => {
                log::error!("Miner error: {e}");
                *state().status.lock() = format!("Error: {e}");
            }
        }
        state().running.store(false, Ordering::SeqCst);
    });
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) {
    state().running.store(false, Ordering::SeqCst);
    *state().status.lock() = "Stopping…".to_string();
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeGetStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let s = state().status.lock().clone();
    env.new_string(s).map(|js| js.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeIsRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    state().running.load(Ordering::SeqCst) as jboolean
}

/// JNI entry point for `NativeFlipperService.nativeIngestFrame`. Called from
/// the Java BLE-receive callback with the raw frame bytes and the paired
/// Flipper's registered ed25519 public key (64 hex chars).
///
/// Returns true if a running node accepted the frame for processing (i.e.
/// `nativeStart` has been called and an account/api_base are on file); an
/// invalid/forged frame still returns true here — it is dropped silently
/// inside `flipper_rx::handle_ble_frame` (the anti-spoof boundary), so the
/// return value is not a validity signal. Returns false only when no node is
/// running yet.
#[no_mangle]
pub extern "C" fn Java_network_hone_app_NativeFlipperService_nativeIngestFrame(
    mut env: JNIEnv,
    _class: JClass,
    frame: JByteArray,
    flipper_pubkey: JString,
) -> jboolean {
    let st = state();

    let api_base = st.api_base.lock().clone();
    let account = st.account.lock().clone();
    if api_base.is_empty() || account.is_empty() {
        log::warn!("nativeIngestFrame: no node running yet (nativeStart not called)");
        return false as jboolean;
    }
    let posting_key = st.posting_key.lock().clone();
    let chain_id = st.chain_id.lock().clone();

    let buf: Vec<u8> = match env.convert_byte_array(&frame) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("nativeIngestFrame: failed to read frame bytes: {e}");
            return false as jboolean;
        }
    };
    let flipper_pk_hex: String = match env.get_string(&flipper_pubkey) {
        Ok(s) => s.into(),
        Err(e) => {
            log::warn!("nativeIngestFrame: failed to read flipper pubkey: {e}");
            return false as jboolean;
        }
    };

    let client = flipper_http_client().clone();
    runtime().spawn(async move {
        flipper_rx::handle_ble_frame(
            buf,
            flipper_pk_hex,
            account,
            posting_key,
            api_base,
            chain_id,
            client,
        ).await;
    });

    true as jboolean
}
