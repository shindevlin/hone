//! Gateway Resolver — URL → store/product resolution, shortcodes.
//! P5-I: ~100 LOC

use crate::chain::Chain;

/// Resolve a BTCPC shortcode or URL path to a store or product.
/// Shortcodes: `@account`, `@account/product`, `btcpc:{account}/{product}`
pub fn resolve(chain: &Chain, input: &str) -> serde_json::Value {
    // Strip leading `@` or `btcpc:` prefix
    let path = input
        .trim_start_matches('@')
        .trim_start_matches("btcpc:");

    let parts: Vec<&str> = path.splitn(2, '/').collect();
    let account = parts[0];
    let product_id = parts.get(1).copied();

    let store_key = format!("storefront:{}", account);
    let store: serde_json::Value = chain.store.state_get(&store_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null);

    if let Some(pid) = product_id {
        let product_key = format!("product:{}:{}", account, pid);
        let product: serde_json::Value = chain.store.state_get(&product_key)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::Value::Null);
        serde_json::json!({
            "account": account,
            "product_id": pid,
            "store": store,
            "product": product,
        })
    } else {
        serde_json::json!({
            "account": account,
            "store": store,
        })
    }
}

/// Resolve a registered name (e.g. "satoshi" → account).
pub fn resolve_name(chain: &Chain, name: &str) -> Option<String> {
    chain.store.state_get(&format!("name_owner:{}", name))
        .and_then(|b| String::from_utf8(b).ok())
}
