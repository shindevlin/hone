mod clock;
mod llm;
mod miner;

use std::sync::{OnceLock, Mutex};
use std::sync::Arc;
use parking_lot::Mutex as PLMutex;
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring, JNI_TRUE, JNI_FALSE};

// ── Global node state ─────────────────────────────────────────────────────────

static NODE: OnceLock<Mutex<Option<NodeHandle>>> = OnceLock::new();

pub(crate) struct NodeHandle {
    pub(crate) runtime:     tokio::runtime::Runtime,
    pub(crate) shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub(crate) status:      Arc<PLMutex<String>>,
}

fn node_cell() -> &'static Mutex<Option<NodeHandle>> {
    NODE.get_or_init(|| Mutex::new(None))
}

// ── JNI: MinerService ────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    account:     JString,
    jwt:         JString,
    api_base:    JString,
    model_id:    JString,
    model_dir:   JString,
    posting_key: JString,
) {
    let account:     String = env.get_string(&account).map(Into::into).unwrap_or_default();
    let jwt:         String = env.get_string(&jwt).map(Into::into).unwrap_or_default();
    let api_base:    String = env.get_string(&api_base).map(Into::into).unwrap_or_default();
    let model_id:    String = env.get_string(&model_id).map(Into::into).unwrap_or_default();
    let model_dir:   String = env.get_string(&model_dir).map(Into::into).unwrap_or_default();
    let posting_key: String = env.get_string(&posting_key).map(Into::into).unwrap_or_default();

    let mut guard = node_cell().lock().unwrap();
    if guard.is_some() {
        return; // already running
    }

    let status = Arc::new(PLMutex::new("Starting…".to_owned()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let cfg = miner::MinerConfig {
        account, jwt, api_base, model_id, model_dir, posting_key,
    };

    let status_clone = status.clone();
    rt.spawn(miner::run(cfg, status_clone, shutdown_rx));

    *guard = Some(NodeHandle { runtime: rt, shutdown_tx: Some(shutdown_tx), status });
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) {
    let mut guard = node_cell().lock().unwrap();
    if let Some(mut handle) = guard.take() {
        if let Some(tx) = handle.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Runtime drops here → all tasks cancelled.
    }
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeGetStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let status = {
        let guard = node_cell().lock().unwrap();
        guard.as_ref()
            .map(|h| h.status.lock().clone())
            .unwrap_or_else(|| "Stopped".to_owned())
    };
    env.new_string(status)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeIsRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let guard = node_cell().lock().unwrap();
    if guard.is_some() { JNI_TRUE } else { JNI_FALSE }
}

// ── JNI: NativeClockService ──────────────────────────────────────────────────

static CLOCK: OnceLock<Mutex<Option<ClockHandle>>> = OnceLock::new();

struct ClockHandle {
    runtime:     tokio::runtime::Runtime,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    status:      Arc<PLMutex<String>>,
}

fn clock_cell() -> &'static Mutex<Option<ClockHandle>> {
    CLOCK.get_or_init(|| Mutex::new(None))
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_NativeClockService_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    account:     JString,
    api_base:    JString,
    chain_id:    JString,
    genesis_ts:  jni::sys::jlong,
    posting_key: JString,
) {
    let account:     String = env.get_string(&account).map(Into::into).unwrap_or_default();
    let api_base:    String = env.get_string(&api_base).map(Into::into).unwrap_or_default();
    let chain_id:    String = env.get_string(&chain_id).map(Into::into).unwrap_or_default();
    let posting_key: String = env.get_string(&posting_key).map(Into::into).unwrap_or_default();
    let genesis_ts = genesis_ts as u64;

    let mut guard = clock_cell().lock().unwrap();
    if guard.is_some() { return; }

    let status = Arc::new(PLMutex::new("Starting…".to_owned()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("clock runtime");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let cfg = clock::ClockConfig { account, api_base, chain_id, genesis_ts, posting_key };
    let status_clone = status.clone();
    rt.spawn(clock::run(cfg, status_clone, shutdown_rx));

    *guard = Some(ClockHandle { runtime: rt, shutdown_tx: Some(shutdown_tx), status });
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_NativeClockService_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) {
    let mut guard = clock_cell().lock().unwrap();
    if let Some(mut handle) = guard.take() {
        if let Some(tx) = handle.shutdown_tx.take() { let _ = tx.send(()); }
    }
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_NativeClockService_nativeGetStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let status = {
        let guard = clock_cell().lock().unwrap();
        guard.as_ref()
            .map(|h| h.status.lock().clone())
            .unwrap_or_else(|| "Stopped".to_owned())
    };
    env.new_string(status)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
