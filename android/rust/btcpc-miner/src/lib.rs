use jni::JNIEnv;
use jni::objects::{JClass, JString, JObject};
use jni::sys::{jstring, jboolean};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod miner;
mod proof;

static RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
static MINER_STATE: OnceCell<Arc<MinerState>> = OnceCell::new();

struct MinerState {
    running: AtomicBool,
    status: Mutex<String>,
}

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("btcpc-miner")
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
        })
    })
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    j_account: JString,
    j_jwt: JString,
    j_api_base: JString,
    j_model_dir: JString,
) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("btcpc-miner"),
    );

    let account: String  = env.get_string(&j_account).map(|s| s.into()).unwrap_or_default();
    let jwt: String      = env.get_string(&j_jwt).map(|s| s.into()).unwrap_or_default();
    let api_base: String = env.get_string(&j_api_base).map(|s| s.into()).unwrap_or_default();
    let model_dir: String= env.get_string(&j_model_dir).map(|s| s.into()).unwrap_or_default();

    let st = state().clone();
    if st.running.swap(true, Ordering::SeqCst) {
        return; // already running
    }
    *st.status.lock() = "Starting…".to_string();

    runtime().spawn(async move {
        *state().status.lock() = "Loading model…".to_string();
        match miner::run_miner(account, jwt, api_base, model_dir, state()).await {
            Ok(_) => {
                *state().status.lock() = "Stopped".to_string();
            }
            Err(e) => {
                log::error!("Miner error: {e}");
                *state().status.lock() = format!("Error: {e}");
            }
        }
        state().running.store(false, Ordering::SeqCst);
    });
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) {
    state().running.store(false, Ordering::SeqCst);
    *state().status.lock() = "Stopping…".to_string();
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeGetStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let s = state().status.lock().clone();
    env.new_string(s).map(|js| js.into_raw()).unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn Java_network_btcpc_app_MinerService_nativeIsRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    state().running.load(Ordering::SeqCst) as jboolean
}
