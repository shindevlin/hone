/*!
Procedural macros for BTCPC smart contracts.

# Attributes

- `#[btcpc_contract]` — on a struct: derives Borsh, adds state load/save logic
- `#[btcpc_impl]` — on an impl block: generates the WASM dispatch table
- `#[init]` — marks the constructor (called once on CONTRACT_DEPLOY)
- `#[call]` — public state-changing method (costs EB)
- `#[view]` — public read-only method (cheaper EB, no state write)
- `#[private]` — only callable by the contract itself (for callbacks)
- `#[callback]` — receives the result of a cross-contract call

The `#[btcpc_impl]` macro on an impl block generates a `__btcpc_dispatch()`
extern "C" function that:
  1. Reads the method name and JSON args from host registers
  2. Loads contract state from storage (for non-view methods)
  3. Dispatches to the annotated method
  4. Saves mutated state back to storage
  5. Writes the return value to the host
*/

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, ItemImpl, ImplItem, ImplItemFn};

/// Mark a struct as a BTCPC smart contract.
/// Derives BorshSerialize, BorshDeserialize, and adds the state key constant.
#[proc_macro_attribute]
pub fn btcpc_contract(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let expanded = quote! {
        #[derive(::btcpc_contract_sdk::BorshSerialize, ::btcpc_contract_sdk::BorshDeserialize)]
        #input

        impl #name {
            const __STATE_KEY: &'static [u8] = b"__state";

            fn __load() -> Self {
                ::btcpc_contract_sdk::env::storage_read(Self::__STATE_KEY)
                    .and_then(|bytes| {
                        <Self as ::btcpc_contract_sdk::BorshDeserialize>::try_from_slice(&bytes).ok()
                    })
                    .unwrap_or_else(|| panic!("Contract state not initialized. Call init first."))
            }

            fn __save(&self) {
                let mut buf = Vec::new();
                ::btcpc_contract_sdk::BorshSerialize::serialize(self, &mut buf).unwrap();
                ::btcpc_contract_sdk::env::storage_write(Self::__STATE_KEY, &buf);
            }
        }
    };

    TokenStream::from(expanded)
}

/// Generate the WASM dispatch table for an impl block.
/// Place immediately before `impl MyContract { ... }`.
#[proc_macro_attribute]
pub fn btcpc_impl(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let impl_block = parse_macro_input!(input as ItemImpl);
    let self_ty = &impl_block.self_ty;

    // Collect all annotated methods
    let mut init_methods: Vec<&ImplItemFn> = Vec::new();
    let mut call_methods: Vec<&ImplItemFn> = Vec::new();
    let mut view_methods: Vec<&ImplItemFn> = Vec::new();

    for item in &impl_block.items {
        if let ImplItem::Fn(method) = item {
            let has_attr = |name: &str| method.attrs.iter().any(|a| a.path().is_ident(name));
            if has_attr("init")     { init_methods.push(method); }
            if has_attr("call")     { call_methods.push(method); }
            if has_attr("view")     { view_methods.push(method); }
        }
    }

    // Build dispatch arms for call methods (load state, mutate, save state)
    let call_arms: Vec<TokenStream2> = call_methods.iter().map(|m| {
        let method_name = &m.sig.ident;
        let name_str = method_name.to_string();
        let args_parse = build_args_parse(m);
        let call_args = build_call_args(m);
        quote! {
            #name_str => {
                let mut contract = <#self_ty>::__load();
                #args_parse
                let result = contract.#method_name(#call_args);
                contract.__save();
                if let Some(val) = to_json_bytes(&result) {
                    ::btcpc_contract_sdk::env::value_return(&val);
                }
            }
        }
    }).collect();

    // Build dispatch arms for view methods (load state, no save)
    let view_arms: Vec<TokenStream2> = view_methods.iter().map(|m| {
        let method_name = &m.sig.ident;
        let name_str = method_name.to_string();
        let args_parse = build_args_parse(m);
        let view_args = build_view_args(m);
        quote! {
            #name_str => {
                let contract = <#self_ty>::__load();
                #args_parse
                let result = contract.#method_name(#view_args);
                if let Some(val) = to_json_bytes(&result) {
                    ::btcpc_contract_sdk::env::value_return(&val);
                }
            }
        }
    }).collect();

    // Build init arms
    let init_arms: Vec<TokenStream2> = init_methods.iter().map(|m| {
        let method_name = &m.sig.ident;
        let name_str = method_name.to_string();
        quote! {
            #name_str => {
                let input = ::btcpc_contract_sdk::env::input();
                let args: ::btcpc_contract_sdk::serde_json::Value =
                    ::btcpc_contract_sdk::serde_json::from_slice(&input).unwrap_or_default();
                let contract = <#self_ty>::#method_name(args);
                contract.__save();
            }
        }
    }).collect();

    let to_json_bytes_fn = to_json_bytes_helper();

    let expanded = quote! {
        #impl_block
        #to_json_bytes_fn

        // The single WASM entry point the runtime calls.
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub extern "C" fn __btcpc_dispatch() {
            // Method name is in register 0, args in register 1 (set by runtime)
            let method_bytes = {
                let len = unsafe { ::btcpc_contract_sdk::env::sys::register_len(0) } as usize;
                let mut buf = vec![0u8; len];
                unsafe { ::btcpc_contract_sdk::env::sys::read_register(0, buf.as_mut_ptr() as u64); }
                buf
            };
            let method = String::from_utf8(method_bytes).unwrap_or_default();

            match method.as_str() {
                #(#init_arms)*
                #(#call_arms)*
                #(#view_arms)*
                other => ::btcpc_contract_sdk::env::panic_str(
                    &format!("Method not found: {}", other)
                ),
            }
        }
    };

    TokenStream::from(expanded)
}

// Marker attributes — the actual behavior is driven by #[btcpc_impl] scanning for them.
#[proc_macro_attribute]
pub fn init(_attr: TokenStream, input: TokenStream) -> TokenStream { input }

#[proc_macro_attribute]
pub fn call(_attr: TokenStream, input: TokenStream) -> TokenStream { input }

#[proc_macro_attribute]
pub fn view(_attr: TokenStream, input: TokenStream) -> TokenStream { input }

#[proc_macro_attribute]
pub fn private(_attr: TokenStream, input: TokenStream) -> TokenStream { input }

#[proc_macro_attribute]
pub fn callback(_attr: TokenStream, input: TokenStream) -> TokenStream { input }

// ── Helpers (simplified; real implementation would fully parse signatures) ───

fn build_args_parse(_m: &ImplItemFn) -> TokenStream2 {
    quote! {
        let __input = ::btcpc_contract_sdk::env::input();
        let __args: ::btcpc_contract_sdk::serde_json::Value =
            ::btcpc_contract_sdk::serde_json::from_slice(&__input).unwrap_or_default();
    }
}

// These are simplified stubs — the real macros would introspect parameter types
// and generate typed deserialization for each parameter.
fn build_call_args(_m: &ImplItemFn) -> TokenStream2 {
    quote! { __args }
}

fn build_view_args(_m: &ImplItemFn) -> TokenStream2 {
    quote! { __args }
}

fn to_json_bytes_helper() -> TokenStream2 {
    quote! {
        fn to_json_bytes<T: ::btcpc_contract_sdk::Serialize>(v: &T) -> Option<Vec<u8>> {
            ::btcpc_contract_sdk::serde_json::to_vec(v).ok()
        }
    }
}
