/*!
# BTCPC Registry Contract

The first system contract deployed on the BTCPC chain.

A simple on-chain key-value store — any BTCPC account can register named
entries, look them up, list all entries for a given account, and delete them.
Useful as general-purpose on-chain metadata storage (IPFS CIDs, public keys,
profile data, custom attributes, etc.).

## Deploy

```bash
btcpc contract deploy registry.wasm --account shindevlin
```

## Usage

```bash
# Register an entry
btcpc contract call <address> register \
  --args '{"key":"profile","value":"ipfs://Qm..."}' \
  --account alice

# Read an entry
btcpc contract view <address> get \
  --args '{"account":"alice","key":"profile"}'

# List all entries for an account
btcpc contract view <address> list \
  --args '{"account":"alice"}'

# Delete an entry
btcpc contract call <address> unregister \
  --args '{"key":"profile"}' \
  --account alice
```
*/

use btcpc_contract_sdk::*;
use borsh::{BorshDeserialize, BorshSerialize};

const MAX_KEY_LEN: usize = 128;
const MAX_VALUE_LEN: usize = 4096;
const MAX_KEYS_PER_ACCOUNT: usize = 64;

// ── Contract state ────────────────────────────────────────────────────────────

#[btcpc_contract]
pub struct Registry {
    /// account_id#key → value
    entries: LookupMap<String, String>,
    /// account_id → list of registered keys (for enumeration)
    account_keys: LookupMap<String, Vec<String>>,
}

// ── Methods ───────────────────────────────────────────────────────────────────

#[btcpc_impl]
impl Registry {
    #[init]
    pub fn new() -> Self {
        Self {
            entries: LookupMap::new(b"e"),
            account_keys: LookupMap::new(b"k"),
        }
    }

    /// Register or update a key-value entry for the calling account.
    #[call]
    pub fn register(&mut self, key: String, value: String) {
        let caller = env::signer();
        require!(key.len() <= MAX_KEY_LEN, "key too long (max 128 chars)");
        require!(!key.is_empty(), "key must not be empty");
        require!(value.len() <= MAX_VALUE_LEN, "value too long (max 4096 chars)");
        require!(!key.contains('#'), "key must not contain '#'");

        let store_key = format!("{}#{}", caller, key);

        let mut keys: Vec<String> = self.account_keys.get(&caller).unwrap_or_default();
        if !keys.contains(&key) {
            require!(
                keys.len() < MAX_KEYS_PER_ACCOUNT,
                "account has reached the maximum number of keys (64)"
            );
            keys.push(key.clone());
            self.account_keys.insert(&caller, &keys);
        }

        self.entries.insert(&store_key, &value);
        log!("[registry] {} set {}={:.40}", caller, key, value);
        emit!(serde_json::json!({
            "standard": "btcpc-registry",
            "event": "register",
            "account": caller,
            "key": key,
        }));
    }

    /// Delete an entry for the calling account.
    #[call]
    pub fn unregister(&mut self, key: String) {
        let caller = env::signer();
        let store_key = format!("{}#{}", caller, key);
        require!(
            self.entries.get(&store_key).is_some(),
            "entry not found"
        );
        self.entries.remove(&store_key);

        let mut keys: Vec<String> = self.account_keys.get(&caller).unwrap_or_default();
        keys.retain(|k| k != &key);
        if keys.is_empty() {
            self.account_keys.remove(&caller);
        } else {
            self.account_keys.insert(&caller, &keys);
        }

        log!("[registry] {} deleted {}", caller, key);
        emit!(serde_json::json!({
            "standard": "btcpc-registry",
            "event": "unregister",
            "account": caller,
            "key": key,
        }));
    }

    /// Read a single entry. Returns null if not found.
    #[view]
    pub fn get(&self, account: String, key: String) -> Option<String> {
        let store_key = format!("{}#{}", account, key);
        self.entries.get(&store_key)
    }

    /// List all keys registered by an account.
    #[view]
    pub fn list_keys(&self, account: String) -> Vec<String> {
        self.account_keys.get(&account).unwrap_or_default()
    }

    /// List all key-value pairs for an account.
    #[view]
    pub fn list(&self, account: String) -> Vec<(String, String)> {
        let keys = self.account_keys.get(&account).unwrap_or_default();
        keys.into_iter()
            .filter_map(|k| {
                let store_key = format!("{}#{}", account, k);
                self.entries.get(&store_key).map(|v| (k, v))
            })
            .collect()
    }

    /// Check if a specific key is registered for an account.
    #[view]
    pub fn has(&self, account: String, key: String) -> bool {
        let store_key = format!("{}#{}", account, key);
        self.entries.get(&store_key).is_some()
    }

    /// Count how many keys an account has registered.
    #[view]
    pub fn count(&self, account: String) -> u64 {
        self.account_keys.get(&account).map(|k| k.len() as u64).unwrap_or(0)
    }
}
