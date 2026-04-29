//! Host-side mock runtime for non-wasm builds and tests.

use crate::types::{AccountId, Balance, Epoch};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct MockEnvState {
    signer: AccountId,
    predecessor: AccountId,
    contract: AccountId,
    epoch: Epoch,
    attached_deposit: Balance,
    balances: HashMap<AccountId, Balance>,
    input: Vec<u8>,
    return_value: Vec<u8>,
    next_promise_id: u64,
}

fn env_state() -> &'static Mutex<MockEnvState> {
    static STATE: OnceLock<Mutex<MockEnvState>> = OnceLock::new();
    STATE.get_or_init(|| {
        let mut s = MockEnvState::default();
        s.signer = "mock.signer".to_string();
        s.predecessor = "mock.predecessor".to_string();
        s.contract = "mock.contract".to_string();
        s.next_promise_id = 1;
        Mutex::new(s)
    })
}

fn storage_state() -> &'static Mutex<HashMap<Vec<u8>, Vec<u8>>> {
    static STORE: OnceLock<Mutex<HashMap<Vec<u8>, Vec<u8>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub mod storage {
    use super::storage_state;

    pub fn write(key: &[u8], value: &[u8]) {
        storage_state().lock().unwrap().insert(key.to_vec(), value.to_vec());
    }

    pub fn read(key: &[u8]) -> Option<Vec<u8>> {
        storage_state().lock().unwrap().get(key).cloned()
    }

    pub fn remove(key: &[u8]) -> bool {
        storage_state().lock().unwrap().remove(key).is_some()
    }

    pub fn has_key(key: &[u8]) -> bool {
        storage_state().lock().unwrap().contains_key(key)
    }
}

pub mod env {
    use super::env_state;
    use crate::types::{AccountId, Balance, Epoch};

    pub fn signer() -> AccountId {
        env_state().lock().unwrap().signer.clone()
    }

    pub fn predecessor() -> AccountId {
        env_state().lock().unwrap().predecessor.clone()
    }

    pub fn current_contract() -> AccountId {
        env_state().lock().unwrap().contract.clone()
    }

    pub fn epoch() -> Epoch {
        env_state().lock().unwrap().epoch
    }

    pub fn attached_deposit() -> Balance {
        env_state().lock().unwrap().attached_deposit
    }

    pub fn set_attached_deposit(amount: Balance) {
        env_state().lock().unwrap().attached_deposit = amount;
    }

    pub fn balance_of(account: &AccountId) -> Balance {
        *env_state().lock().unwrap().balances.get(account).unwrap_or(&0)
    }

    pub fn transfer(to: &AccountId, amount: Balance) -> bool {
        let mut state = env_state().lock().unwrap();
        let from = state.contract.clone();
        let from_balance = *state.balances.get(&from).unwrap_or(&0);
        if from_balance < amount {
            return false;
        }
        state.balances.insert(from.clone(), from_balance - amount);
        let to_balance = *state.balances.get(to).unwrap_or(&0);
        state.balances.insert(to.clone(), to_balance + amount);
        true
    }

    pub fn input() -> Vec<u8> {
        env_state().lock().unwrap().input.clone()
    }

    pub fn set_return_value(value: Vec<u8>) {
        env_state().lock().unwrap().return_value = value;
    }

    pub fn promise_create(
        _contract: &str,
        _method: &str,
        _args: &[u8],
        _amount: Balance,
        _gas: u64,
    ) -> u64 {
        let mut state = env_state().lock().unwrap();
        let id = state.next_promise_id;
        state.next_promise_id = state.next_promise_id.saturating_add(1);
        id
    }
}

