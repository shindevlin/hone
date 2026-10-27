// uniffi scaffolding must live at the crate root (generates crate-root UniFfiTag).
uniffi::setup_scaffolding!();

mod bridge;   // uniffi typed interface (the NEW Kotlin surface)
mod chain;
mod clock;
mod llm;
mod miner;
mod net;
mod node;
mod sensors;
mod store;

use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use parking_lot::Mutex as PLMutex;
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jlong, jstring, JNI_TRUE, JNI_FALSE};

use node::{NodeConfig, NodeHandle};

// ── Global node state ─────────────────────────────────────────────────────────

static NODE: OnceLock<Mutex<Option<RunningNode>>> = OnceLock::new();

struct RunningNode {
    handle:      NodeHandle,
    runtime:     tokio::runtime::Runtime,
}

fn node_cell() -> &'static Mutex<Option<RunningNode>> {
    NODE.get_or_init(|| Mutex::new(None))
}

// ── JNI: MinerService ────────────────────────────────────────────────────────
//
// Java signature (updated):
//   nativeStart(account, postingKey, chainId, genesisTs, dataDir,
//               modelId, modelDir, bootstrapPeers, p2pPort, isMiner, isClock)

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeStart(
    mut env:  JNIEnv,
    _class:   JClass,
    account:         JString,
    posting_key:     JString,
    chain_id:        JString,
    genesis_ts:      jlong,
    data_dir:        JString,
    model_id:        JString,
    model_dir:       JString,
    bootstrap_peers: JString,
    p2p_port:        jlong,
    is_miner:        jboolean,
    is_clock:        jboolean,
) {
    let mut guard = node_cell().lock().unwrap();
    if guard.is_some() { return; }

    macro_rules! str { ($j:expr) => { env.get_string(&$j).map(String::from).unwrap_or_default() } }

    let cfg = NodeConfig {
        account:         str!(account),
        posting_key:     str!(posting_key),
        chain_id:        str!(chain_id),
        genesis_ts:      genesis_ts as u64,
        data_dir:        str!(data_dir),
        model_id:        str!(model_id),
        model_dir:       str!(model_dir),
        bootstrap_peers: str!(bootstrap_peers)
            .split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        p2p_port:        p2p_port as u16,
        is_miner:        is_miner == JNI_TRUE,
        is_clock:        is_clock == JNI_TRUE,
    };

    let status = Arc::new(PLMutex::new("Starting…".to_owned()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .thread_name("hone-node")
        .build()
        .expect("tokio runtime");

    let status_clone = status.clone();
    let handle = rt.block_on(async move {
        node::start(cfg, status_clone).await
    });

    match handle {
        Ok(h) => { *guard = Some(RunningNode { handle: h, runtime: rt }); }
        Err(e) => { tracing::error!("node start failed: {}", e); }
    }
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeStop(
    _env: JNIEnv, _class: JClass,
) {
    let mut guard = node_cell().lock().unwrap();
    if let Some(running) = guard.take() {
        // Signal all tasks to shut down.
        let _ = running.handle.shutdown_tx.send(());
        // Runtime drop cancels all spawned tasks.
        drop(running);
    }
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeGetStatus(
    env: JNIEnv, _class: JClass,
) -> jstring {
    let s = {
        let guard = node_cell().lock().unwrap();
        guard.as_ref()
            .map(|r| r.handle.status.lock().clone())
            .unwrap_or_else(|| "Stopped".to_owned())
    };
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeIsRunning(
    _env: JNIEnv, _class: JClass,
) -> jboolean {
    if node_cell().lock().unwrap().is_some() { JNI_TRUE } else { JNI_FALSE }
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeGetBalance(
    mut env: JNIEnv, _class: JClass,
    account: JString,
) -> jlong {
    let account: String = env.get_string(&account).map(Into::into).unwrap_or_default();
    let guard = node_cell().lock().unwrap();
    guard.as_ref()
        .map(|r| r.handle.chain.store.get_balance(&account, hone_types::NATIVE_TOKEN) as i64)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_MinerService_nativeGetEpoch(
    _env: JNIEnv, _class: JClass,
) -> jlong {
    let guard = node_cell().lock().unwrap();
    guard.as_ref()
        .map(|r| r.handle.chain.current_epoch() as i64)
        .unwrap_or(0)
}

// ── JNI: NativeClockService ──────────────────────────────────────────────────

static CLOCK_NODE: OnceLock<Mutex<Option<RunningNode>>> = OnceLock::new();

fn clock_cell() -> &'static Mutex<Option<RunningNode>> {
    CLOCK_NODE.get_or_init(|| Mutex::new(None))
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_NativeClockService_nativeStart(
    mut env: JNIEnv,
    _class:  JClass,
    account:          JString,
    api_base:         JString,
    chain_id:         JString,
    genesis_ts:       jlong,
    posting_key:      JString,
    bootstrap_peers:  JString,
) {
    let mut guard = clock_cell().lock().unwrap();
    if guard.is_some() { return; }

    macro_rules! str { ($j:expr) => { env.get_string(&$j).map(String::from).unwrap_or_default() } }

    let _api_base = str!(api_base);

    let cfg = NodeConfig {
        account:         str!(account),
        posting_key:     str!(posting_key),
        chain_id:        str!(chain_id),
        genesis_ts:      genesis_ts as u64,
        data_dir:        "/data/data/network.hone.app/files/hone-clock".to_owned(),
        model_id:        String::new(),
        model_dir:       String::new(),
        bootstrap_peers: str!(bootstrap_peers)
            .split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        p2p_port:        6943,
        is_miner:        false,
        is_clock:        true,
    };

    let status = Arc::new(PLMutex::new("Starting…".to_owned()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("hone-clock")
        .build()
        .expect("tokio runtime");

    let status_clone = status.clone();
    let handle = rt.block_on(async move {
        node::start(cfg, status_clone).await
    });

    match handle {
        Ok(h) => { *guard = Some(RunningNode { handle: h, runtime: rt }); }
        Err(e) => { tracing::error!("clock start failed: {}", e); }
    }
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_NativeClockService_nativeStop(
    _env: JNIEnv, _class: JClass,
) {
    let mut guard = clock_cell().lock().unwrap();
    if let Some(running) = guard.take() {
        let _ = running.handle.shutdown_tx.send(());
        drop(running);
    }
}

#[no_mangle]
pub extern "C" fn Java_network_hone_app_NativeClockService_nativeGetStatus(
    env: JNIEnv, _class: JClass,
) -> jstring {
    let s = {
        let guard = clock_cell().lock().unwrap();
        guard.as_ref()
            .map(|r| r.handle.status.lock().clone())
            .unwrap_or_else(|| "Stopped".to_owned())
    };
    env.new_string(s).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}

// ── JNI: NativeSensorService ─────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn Java_network_hone_app_NativeSensorService_nativeSubmitReading(
    mut env:        JNIEnv,
    _class:         JClass,
    sensor_id:      JString,
    sensor_type:    JString,
    primary_value:  jni::sys::jdouble,
    values_json:    JString,  // e.g. {"x":1.2,"y":0.3,"z":9.8} or {"lat":53.3,"lon":-6.2}
    unit:           JString,
) -> jboolean {
    macro_rules! str { ($j:expr) => { env.get_string(&$j).map(String::from).unwrap_or_default() } }

    let guard = node_cell().lock().unwrap();
    let Some(running) = guard.as_ref() else { return JNI_FALSE; };

    let reading = sensors::SensorReading {
        sensor_id:     str!(sensor_id),
        sensor_type:   str!(sensor_type),
        primary_value,
        values_json:   str!(values_json),
        unit:          str!(unit),
        owner:   running.handle.chain.node_id.clone(),
        epoch:   running.handle.chain.current_epoch(),
    };

    let chain   = running.handle.chain.clone();
    let cmd_tx  = running.handle.cmd_tx.clone();

    // Dispatch sensor submission onto the runtime.
    running.runtime.spawn(async move {
        sensors::submit(reading, chain, cmd_tx).await;
    });

    JNI_TRUE
}

// ── Logging: tracing -> Android logcat (tag "hone") ─────────────────────────
#[cfg(target_os = "android")]
mod logcat {
    use std::ffi::CString;
    #[link(name = "log")]
    extern "C" {
        fn __android_log_write(prio: i32, tag: *const std::os::raw::c_char, text: *const std::os::raw::c_char) -> i32;
    }
    pub struct Writer;
    impl std::io::Write for Writer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(buf).trim_end().replace('\0', "");
            if let (Ok(tag), Ok(msg)) = (CString::new("hone"), CString::new(text)) {
                // 4 = ANDROID_LOG_INFO
                unsafe { __android_log_write(4, tag.as_ptr(), msg.as_ptr()); }
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
}

/// Route `tracing` output to logcat once per process (no-op off Android).
pub(crate) fn init_logcat() {
    #[cfg(target_os = "android")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            use tracing_subscriber::prelude::*;
            // HONE_LOG_DEBUG_GOSSIP=1 (or a hone-node/debug_gossip file) adds gossipsub debug
            let targets = tracing_subscriber::filter::Targets::new()
                .with_default(tracing::Level::INFO)
                .with_target("libp2p_gossipsub", tracing::Level::DEBUG);
            let _ = tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer()
                    .with_writer(|| logcat::Writer)
                    .with_ansi(false)
                    .without_time())
                .with(targets)
                .try_init();
        });
    }
}
