/*!
Procedural macros for HONE smart contracts.

# Attributes

- `#[hone_contract]` — on a struct: derives Borsh, adds state load/save logic
- `#[hone_impl]` — on an impl block: generates the WASM dispatch table
- `#[init]` — marks the constructor (called once on CONTRACT_DEPLOY)
- `#[call]` — public state-changing method (costs EB)
- `#[view]` — public read-only method (cheaper EB, no state write)
- `#[private]` — only callable by the contract itself (for callbacks)
- `#[callback]` — receives the result of a cross-contract call

The `#[hone_impl]` macro on an impl block generates a `__hone_dispatch()`
extern "C" function that:
  1. Reads the method name and JSON args from host registers
  2. Loads contract state from storage (for non-view methods)
  3. Dispatches to the annotated method with typed per-parameter deserialization
  4. Saves mutated state back to storage
  5. Writes the return value to the host
*/

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, ItemImpl, ImplItem, ImplItemFn, FnArg, Pat};

/// Mark a struct as a HONE smart contract.
#[proc_macro_attribute]
pub fn hone_contract(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let expanded = quote! {
        #[derive(::hone_contract_sdk::BorshSerialize, ::hone_contract_sdk::BorshDeserialize)]
        #input

        impl #name {
            const __STATE_KEY: &'static [u8] = b"__state";

            fn __load() -> Self {
                ::hone_contract_sdk::env::storage_read(Self::__STATE_KEY)
                    .and_then(|bytes| {
                        <Self as ::hone_contract_sdk::BorshDeserialize>::try_from_slice(&bytes).ok()
                    })
                    .unwrap_or_else(|| ::hone_contract_sdk::env::panic_str(
                        "Contract state not initialized — call init first"
                    ))
            }

            fn __save(&self) {
                let mut buf = ::hone_contract_sdk::__private::vec![];
                ::hone_contract_sdk::BorshSerialize::serialize(self, &mut buf).unwrap();
                ::hone_contract_sdk::env::storage_write(Self::__STATE_KEY, &buf);
            }
        }
    };

    TokenStream::from(expanded)
}

/// Generate the WASM dispatch table for an impl block.
#[proc_macro_attribute]
pub fn hone_impl(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let impl_block = parse_macro_input!(input as ItemImpl);
    let self_ty = &impl_block.self_ty;

    let mut init_methods: Vec<&ImplItemFn> = Vec::new();
    let mut call_methods: Vec<&ImplItemFn> = Vec::new();
    let mut view_methods: Vec<&ImplItemFn> = Vec::new();

    for item in &impl_block.items {
        if let ImplItem::Fn(method) = item {
            let has_attr = |name: &str| method.attrs.iter().any(|a| a.path().is_ident(name));
            if has_attr("init")  { init_methods.push(method); }
            if has_attr("call")  { call_methods.push(method); }
            if has_attr("view")  { view_methods.push(method); }
        }
    }

    let call_arms: Vec<TokenStream2> = call_methods.iter().map(|m| {
        let method_name = &m.sig.ident;
        let name_str = method_name.to_string();
        let (parse_stmts, call_args) = build_dispatch(m);
        quote! {
            #name_str => {
                let mut contract = <#self_ty>::__load();
                #parse_stmts
                let result = contract.#method_name(#call_args);
                contract.__save();
                if let Some(val) = __to_json_bytes(&result) {
                    ::hone_contract_sdk::env::value_return(&val);
                }
            }
        }
    }).collect();

    let view_arms: Vec<TokenStream2> = view_methods.iter().map(|m| {
        let method_name = &m.sig.ident;
        let name_str = method_name.to_string();
        let (parse_stmts, call_args) = build_dispatch(m);
        quote! {
            #name_str => {
                let contract = <#self_ty>::__load();
                #parse_stmts
                let result = contract.#method_name(#call_args);
                if let Some(val) = __to_json_bytes(&result) {
                    ::hone_contract_sdk::env::value_return(&val);
                }
            }
        }
    }).collect();

    let init_arms: Vec<TokenStream2> = init_methods.iter().map(|m| {
        let method_name = &m.sig.ident;
        let name_str = method_name.to_string();
        let (parse_stmts, call_args) = build_dispatch(m);
        quote! {
            #name_str => {
                #parse_stmts
                let contract = <#self_ty>::#method_name(#call_args);
                contract.__save();
            }
        }
    }).collect();

    let expanded = quote! {
        #impl_block

        #[allow(unused)]
        fn __to_json_bytes<T: ::hone_contract_sdk::Serialize>(v: &T) -> Option<::hone_contract_sdk::__private::Vec<u8>> {
            ::hone_contract_sdk::serde_json::to_vec(v).ok()
        }

        // Single WASM entry point called by the runtime.
        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub extern "C" fn __hone_dispatch() {
            let method = ::hone_contract_sdk::env::read_method_name();
            match method.as_str() {
                #(#init_arms)*
                #(#call_arms)*
                #(#view_arms)*
                other => ::hone_contract_sdk::env::panic_str(
                    &::hone_contract_sdk::__private::format!("method not found: {}", other)
                ),
            }
        }
    };

    TokenStream::from(expanded)
}

// ── Per-parameter typed deserialization ───────────────────────────────────────

/// Returns (parse_statements, call_argument_list) for a given method.
/// Skips `self` / `&self` / `&mut self`.
fn build_dispatch(m: &ImplItemFn) -> (TokenStream2, TokenStream2) {
    // Collect user parameters (skip self receiver).
    let params: Vec<(proc_macro2::Ident, Box<syn::Type>)> = m.sig.inputs.iter().filter_map(|arg| {
        if let FnArg::Typed(pat_type) = arg {
            if let Pat::Ident(pat_ident) = &*pat_type.pat {
                return Some((pat_ident.ident.clone(), pat_type.ty.clone()));
            }
        }
        None
    }).collect();

    if params.is_empty() {
        // No args — no need to parse input.
        return (quote! {}, quote! {});
    }

    let input_parse = quote! {
        let __input = ::hone_contract_sdk::env::input();
        let __args: ::hone_contract_sdk::serde_json::Value =
            ::hone_contract_sdk::serde_json::from_slice(&__input).unwrap_or_default();
    };

    let mut per_param: Vec<TokenStream2> = Vec::new();
    let mut call_idents: Vec<proc_macro2::Ident> = Vec::new();

    for (ident, ty) in &params {
        let key_str = ident.to_string();
        per_param.push(quote! {
            let #ident: #ty = ::hone_contract_sdk::serde_json::from_value(
                __args[#key_str].clone()
            ).unwrap_or_default();
        });
        call_idents.push(ident.clone());
    }

    let parse_stmts = quote! {
        #input_parse
        #(#per_param)*
    };
    let call_args = quote! { #(#call_idents),* };

    (parse_stmts, call_args)
}

// ── Marker attributes ─────────────────────────────────────────────────────────

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
