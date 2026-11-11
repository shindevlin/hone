//! Contract replay test: runs a fixed set of contracts and calls and compares
//! every observable output (success, return value, gas used, storage writes and
//! deletes, transfers, events, logs, error class) to a golden file.
//!
//! Contract execution must be byte-identical across nodes, so any wasmtime or
//! Cranelift upgrade has to keep this test green. The golden file was produced
//! on wasmtime 23. To regenerate it on purpose:
//!   HONE_REPLAY_BLESS=1 cargo test -p hone-contract-runtime --test replay
use base64::Engine as _;
use hone_contract_runtime::{
    derive_contract_address, CallRequest, CallResult, DeployRequest,
};
use hone_contract_runtime::state::{ContractState, StorageKv};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const GAS: u64 = 300_000_000_000;
const REGISTRY_WASM: &[u8] = include_bytes!("../../hone-node/contracts/registry.wasm");

fn error_class(e: &str) -> String {
    let l = e.to_lowercase();
    for (needle, class) in [
        ("fuel", "trap:out_of_fuel"),
        ("integer overflow", "trap:int_overflow"),
        ("divide by zero", "trap:div_by_zero"),
        ("unreachable", "trap:unreachable"),
        ("out of bounds", "trap:out_of_bounds"),
    ] {
        if l.contains(needle) { return class.to_string(); }
    }
    // wasmtime renders traps as "error while executing at wasm backtrace: ...";
    // the backtrace text is version-specific and never reaches chain state
    // (hone-node only uses it in the rejection message), so compare the class.
    if l.contains("wasm backtrace") || l.contains("wasm trap") {
        return "trap".to_string();
    }
    e.to_string()
}

/// Canonical, order-independent view of a CallResult.
fn canon(r: &CallResult) -> Value {
    let mut writes: Vec<(String, String)> = r.storage_writes.iter()
        .map(|(k, v)| (k.clone(), format!("{}:{}", v.len(), hex::encode(Sha256::digest(v)))))
        .collect();
    writes.sort();
    let mut deletes = r.storage_deletes.clone();
    deletes.sort();
    json!({
        "success": r.success,
        "result": r.result,
        "gas_used": r.gas_used,
        "storage_writes": writes,
        "storage_deletes": deletes,
        "pending_transfers": r.pending_transfers.iter().map(|(a, n)| json!([a, n.to_string()])).collect::<Vec<_>>(),
        "events": r.events,
        "logs": r.logs,
        "error": r.error.as_deref().map(error_class),
    })
}

fn apply(storage: &mut StorageKv, r: &CallResult) {
    if !r.success { return; }
    for (k, v) in &r.storage_writes { storage.committed.insert(hex::decode(k).unwrap(), v.clone()); }
    for k in &r.storage_deletes { storage.committed.remove(&hex::decode(k).unwrap()); }
}

fn call(cid: &str, storage: &StorageKv, method: &str, args: Value, signer: &str, gas: u64) -> CallResult {
    hone_contract_runtime::executor::execute_call(
        CallRequest {
            contract_id: cid.to_string(), method: method.to_string(), args,
            signer: signer.to_string(), predecessor: signer.to_string(),
            attached_deposit: 0, gas, epoch: 1,
        },
        ContractState::new(cid.to_string(), storage.clone(), 0),
    )
}

fn registry_cases(out: &mut BTreeMap<String, Value>) {
    let deploy = hone_contract_runtime::executor::execute_deploy(
        DeployRequest {
            deployer: "shindevlin".into(),
            wasm_b64: base64::engine::general_purpose::STANDARD.encode(REGISTRY_WASM),
            init_method: Some("new".into()), init_args: Some(json!({})),
            gas: GAS, epoch: 1, nonce: 1,
        },
        StorageKv::default(),
    );
    let cid = derive_contract_address("shindevlin", 1, 1);
    out.insert("registry/00_deploy".into(), canon(&deploy));
    let mut st = StorageKv::default();
    apply(&mut st, &deploy);
    let steps: Vec<(&str, &str, Value, &str, u64)> = vec![
        ("01_register_profile", "register", json!({"key": "profile", "value": "ipfs://Qmtest"}), "alice", GAS),
        ("02_register_second", "register", json!({"key": "k2", "value": "v2"}), "alice", GAS),
        ("03_register_bob", "register", json!({"key": "profile", "value": "bob-data"}), "bob", GAS),
        ("04_overwrite", "register", json!({"key": "k2", "value": "v2b"}), "alice", GAS),
        ("05_get", "get", json!({"account": "alice", "key": "profile"}), "carol", GAS),
        ("06_get_missing", "get", json!({"account": "alice", "key": "nope"}), "carol", GAS),
        ("07_list_keys", "list_keys", json!({"account": "alice"}), "carol", GAS),
        ("08_count", "count", json!({"account": "alice"}), "carol", GAS),
        ("09_has", "has", json!({"account": "bob", "key": "profile"}), "carol", GAS),
        ("10_bad_args", "register", json!({}), "alice", GAS),
        ("11_unknown_method", "no_such_method", json!({}), "alice", GAS),
        ("12_out_of_gas", "register", json!({"key": "x", "value": "y"}), "alice", 1_000),
    ];
    for (name, method, args, signer, gas) in steps {
        let r = call(&cid, &st, method, args, signer, gas);
        apply(&mut st, &r);
        out.insert(format!("registry/{name}"), canon(&r));
    }
}

/// Module that runs `body` in __hone_dispatch and stores 64 bytes from memory
/// offset 64 under key "r" via storage_write.
fn module(extra: &str, body: &str) -> Vec<u8> {
    let wat = format!(r#"(module
  (import "env" "storage_write" (func $sw (param i64 i64 i64 i64) (result i64)))
  (memory (export "memory") 2)
  (data (i32.const 0) "r")
  {extra}
  (func (export "__hone_dispatch")
    {body}
    (drop (call $sw (i64.const 0) (i64.const 1) (i64.const 64) (i64.const 64)))))"#);
    wat::parse_str(&wat).expect("valid wat")
}

fn big_br_table(n: usize) -> String {
    let mut s = String::from("(func $pick (param $i i32) (result i32)\n(block $d\n");
    for k in (0..n).rev() { s.push_str(&format!("(block $b{k}\n")); }
    s.push_str("(br_table");
    for k in 0..n { s.push_str(&format!(" $b{k}")); }
    s.push_str(" $d (local.get $i)))\n");
    for k in 0..n {
        s.push_str(&format!("(return (i32.const {}))\n", ((k * k) ^ 0x5a5a) as i32));
        if k + 1 < n { s.push_str(")\n"); }
    }
    s.push_str(")\n(i32.const -1))");
    s
}

fn wat_cases(out: &mut BTreeMap<String, Value>) {
    let cases: Vec<(&str, Vec<u8>, u64)> = vec![
        ("simd_f64x2_splat", module("", r#"
            (v128.store (i32.const 64) (f64x2.add (f64x2.splat (f64.const 1.5)) (f64x2.splat (f64.const 2.25))))
            (v128.store (i32.const 80) (f64x2.mul (f64x2.splat (f64.const -0.1)) (f64x2.splat (f64.const 3.0))))
            (v128.store (i32.const 96) (i32x4.mul (i32x4.splat (i32.const 0x7fffffff)) (i32x4.splat (i32.const 3))))
            (v128.store (i32.const 112) (f32x4.sqrt (f32x4.splat (f32.const 2.0))))"#), GAS),
        ("nan_canonicalization", module("", r#"
            (f64.store (i32.const 64) (f64.div (f64.const 0) (f64.const 0)))
            (f64.store (i32.const 72) (f64.sqrt (f64.const -1)))
            (f32.store (i32.const 80) (f32.div (f32.const 0) (f32.const 0)))
            (f64.store (i32.const 88) (f64.add (f64.const nan:0x4) (f64.const 1)))
            (f64.store (i32.const 96) (f64.min (f64.const -0) (f64.const 0)))
            (i64.store (i32.const 104) (i64.trunc_sat_f64_s (f64.const 1e300)))
            (f64.store (i32.const 112) (f64.nearest (f64.const 2.5)))"#), GAS),
        ("big_br_table", module(&big_br_table(1500), r#"
            (local $i i32) (local $acc i64)
            (block $done (loop $l
              (br_if $done (i32.gt_u (local.get $i) (i32.const 1502)))
              (local.set $acc (i64.add (i64.mul (local.get $acc) (i64.const 31))
                                       (i64.extend_i32_s (call $pick (local.get $i)))))
              (local.set $i (i32.add (local.get $i) (i32.const 1)))
              (br $l)))
            (i64.store (i32.const 64) (local.get $acc))"#), GAS),
        ("fuel_loop", module("", r#"
            (local $i i64) (local $x i64)
            (local.set $x (i64.const 1))
            (block $done (loop $l
              (br_if $done (i64.ge_u (local.get $i) (i64.const 100000)))
              (local.set $x (i64.xor (i64.mul (local.get $x) (i64.const 6364136223846793005)) (local.get $i)))
              (local.set $i (i64.add (local.get $i) (i64.const 1)))
              (br $l)))
            (i64.store (i32.const 64) (local.get $x))"#), GAS),
        ("out_of_fuel", module("", r#"(loop $l (br $l))"#), 50_000),
        ("trap_int_overflow", module("", r#"(drop (i64.div_s (i64.const 0x8000000000000000) (i64.const -1)))"#), GAS),
        ("trap_div_by_zero", module("", r#"(drop (i32.div_u (i32.const 1) (i32.const 0)))"#), GAS),
        ("trap_oob", module("", r#"(drop (i64.load (i32.const 0x7fffffff)))"#), GAS),
        ("trap_unreachable", module("", "(unreachable)"), GAS),
        ("bulk_memory_grow", module("", r#"
            (memory.fill (i32.const 64) (i32.const 0xab) (i32.const 64))
            (memory.copy (i32.const 72) (i32.const 0) (i32.const 1))
            (i32.store (i32.const 80) (memory.grow (i32.const 3)))
            (i32.store (i32.const 84) (memory.size))"#), GAS),
    ];
    for (name, wasm, gas) in cases {
        let mut st = StorageKv::default();
        st.committed.insert(b"__wasm".to_vec(), wasm);
        let r = call("honesc_replay", &st, "run", json!({}), "alice", gas);
        out.insert(format!("wat/{name}"), canon(&r));
    }
}

#[test]
fn contract_replay_matches_golden() {
    let mut out = BTreeMap::new();
    registry_cases(&mut out);
    wat_cases(&mut out);
    let actual = serde_json::to_string_pretty(&out).unwrap() + "\n";
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/replay_golden.json");
    if std::env::var("HONE_REPLAY_BLESS").as_deref() == Ok("1") {
        std::fs::write(path, &actual).unwrap();
        return;
    }
    let golden: BTreeMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(path).expect("golden file")).unwrap();
    let mut diffs = Vec::new();
    for (k, v) in &out {
        if golden.get(k) != Some(v) {
            diffs.push(format!("{k}:\n  golden: {}\n  actual: {}", golden.get(k).map(|g| g.to_string()).unwrap_or("<missing>".into()), v));
        }
    }
    for k in golden.keys() { if !out.contains_key(k) { diffs.push(format!("{k}: missing from run")); } }
    assert!(diffs.is_empty(), "contract replay differs from golden:\n{}", diffs.join("\n"));
}
