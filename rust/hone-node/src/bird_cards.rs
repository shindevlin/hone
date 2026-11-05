//! Bird names and cards (gated by BIRD_CARDS_EPOCH; Josh 2026-11-04).
//!
//! `bird_id` (defined by the voiceprint pipeline) is a 64-hex hash of species, res-7 cell,
//! quantised call centroid and first night. A seat reports it with `BIRD_OBSERVE` (seat
//! key); the first report records the bird. The account owning that seat may name it once
//! (`BIRD_NAME`, first valid name wins), then mint its unique card (`BIRD_CARD_CLAIM`) and
//! send it (`BIRD_CARD_SEND`). named_by is the account itself (no pseudonyms). When two
//! bird_ids merge, the earliest name wins and both cards stay (merged_into pointer).
//!
//! State: bird_first:{bird}, bird_name:{bird}, bird_merged:{bird} -> into,
//! bird_card_of:{bird} -> card, bird_card:{card}, bird_card_owner:{owner}:{card}.

use anyhow::{bail, Result};
use hone_types::LedgerEntry;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::chain::Chain;

pub fn activation_epoch() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_EPOCH.with(|c| c.get()) { return v; }
    hone_types::BIRD_CARDS_EPOCH
}
#[cfg(test)]
thread_local! {
    pub static TEST_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Profanity / blocklist hook: substrings refused in names (lowercase). Extend here.
pub const BLOCKLIST: &[&str] = &["fuck", "shit", "cunt", "bitch", "nazi", "dick", "pussy", "whore", "slut"];

pub fn valid_name(n: &str) -> bool {
    let len = n.chars().count();
    (1..=24).contains(&len) && n.trim() == n && !n.contains("  ")
        && n.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-')
        && n.chars().any(|c| c.is_alphabetic())
}
pub fn blocked(n: &str) -> bool { let l = n.to_lowercase(); BLOCKLIST.iter().any(|w| l.contains(w)) }

/// Species whose voiceprints identify individuals well enough to name and merge them:
/// `naming_allowed` from hone-voiceprint eval/gates.json (sha256 e1aa9f1060b86a6d..., commit
/// b7ab0f34). Updating the gate table is a release, like any consensus constant.
pub const NAMING_ALLOWED: &[&str] = &["Athene noctua"];
pub const NAMING_GATES_SHA256_PREFIX: &str = "e1aa9f1060b86a6d";
pub fn naming_allowed(species: &str) -> bool { NAMING_ALLOWED.contains(&species) }

/// Uniqueness key of a name (Josh 2026-11-04): case-folded, accents stripped, Unicode
/// confusables skeleton, whitespace collapsed. "Bób", "BOB" and "bob" share one key.
pub fn norm_name(n: &str) -> String {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    use unicode_security::confusable_detection::skeleton;
    // The skeleton is case-sensitive (Latin I -> l, Cyrillic в -> ʙ), so bring every name to
    // one case first: upper-case, skeleton, then lower-case. The key is never displayed.
    let strip = |s: &str| -> String { s.nfd().filter(|c| !is_combining_mark(*c)).collect() };
    let up = strip(n).to_uppercase();
    let sk = strip(&skeleton(&up).collect::<String>().to_lowercase());
    sk.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// H3 res-6 parent (about town scale) of a res-7 cell.
pub fn res6(cell7: &str) -> Option<String> {
    let c = u64::from_str_radix(cell7, 16).ok().and_then(|v| h3o::CellIndex::try_from(v).ok())?;
    c.parent(h3o::Resolution::Six).map(|p| format!("{:x}", u64::from(p)))
}

/// `bird_name_key:{species}:{res6}:{norm}` -> bird_id. Names are permanent: a merged-away or
/// split name keeps its key, so it is never freed.
pub fn name_key(species: &str, res6: &str, name: &str) -> String { format!("bird_name_key:{species}:{res6}:{}", norm_name(name)) }
fn name_key_for(chain: &Chain, bird: &str, name: &str) -> Option<String> {
    let f = first(chain, bird)?;
    Some(name_key(f["species"].as_str()?, &res6(f["cell_res7"].as_str()?)?, name))
}

/// Merge authority (eval/MERGE_AUTHORITY.md, Josh 2026-11-04).
pub const MERGE_CONSENT_EPOCHS: u64 = 2_880;
pub const MAX_MERGES_PER_WINDOW: u64 = 4;
pub const MERGE_WINDOW_EPOCHS: u64 = 120;

fn is_hex(s: &str, n: usize) -> bool { s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) }
fn get_json(chain: &Chain, k: &str) -> Option<Value> { chain.store.state_get(k).and_then(|b| serde_json::from_slice(&b).ok()) }
fn set_json(chain: &Chain, k: &str, v: &Value) -> Result<()> { chain.store.state_set(k, &serde_json::to_vec(v)?)?; Ok(()) }

pub fn card_id(bird: &str) -> String { hex::encode(&Sha256::digest(format!("hone-bird-card:{bird}").as_bytes())[..16]) }
pub fn first(chain: &Chain, bird: &str) -> Option<Value> { get_json(chain, &format!("bird_first:{bird}")) }
pub fn name_of(chain: &Chain, bird: &str) -> Option<Value> { get_json(chain, &format!("bird_name:{bird}")) }
pub fn card(chain: &Chain, card: &str) -> Option<Value> { get_json(chain, &format!("bird_card:{card}")) }

/// The account owning the seat that first reported `bird` (seat transfers follow the key).
pub fn first_owner(chain: &Chain, bird: &str) -> Option<String> {
    let f = first(chain, bird)?;
    let pk = f["seat_pubkey"].as_str()?;
    let seat = crate::sensor_seat::seat_for_pubkey(chain, pk).unwrap_or_else(|| f["seat"].as_str().unwrap_or_default().to_owned());
    Some(crate::usefulness::owner_of(&seat))
}

pub fn check(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    match entry {
        LedgerEntry::BirdObserve { seat, bird_id, species, cell_res7, first_night, signed_by, .. } => {
            if signed_by != seat { bail!("signed_by must be the seat"); }
            if hone_types::parse_device_clock_id(seat).is_none() { bail!("seat must be owner@<seat pubkey>"); }
            if !is_hex(bird_id, 64) { bail!("bird_id must be 64 lowercase hex"); }
            if species.is_empty() || species.len() > 64 { bail!("species must be 1..64 chars"); }
            if !is_hex(cell_res7, 15) || !cell_res7.starts_with('8') { bail!("cell_res7 must be an H3 index"); }
            if first_night.is_empty() || first_night.len() > 16 { bail!("first_night must be 1..16 chars"); }
        }
        LedgerEntry::BirdName { bird_id, name, signed_by, .. } => {
            if first(chain, bird_id).is_none() { bail!("unknown bird '{}'", bird_id); }
            if chain.store.state_get(&format!("bird_merged:{bird_id}")).is_some() { bail!("bird was merged; name the surviving id"); }
            if first_owner(chain, bird_id).as_deref() != Some(signed_by.as_str()) { bail!("only the owner of the first-reporting seat can name this bird"); }
            if name_of(chain, bird_id).is_some() { bail!("bird already named"); }
            let sp = first(chain, bird_id).and_then(|f| f["species"].as_str().map(str::to_owned)).unwrap_or_default();
            if !naming_allowed(&sp) { bail!("naming is not allowed for species '{}'", sp); }
            let Some(nk) = name_key_for(chain, bird_id, name) else { bail!("bird cell has no res-6 parent") };
            if chain.store.state_get(&nk).is_some() { bail!("a {} named like '{}' already exists in this area", sp, name); }
            if !valid_name(name) { bail!("name: 1-24 letters, spaces, apostrophes or hyphens"); }
            if blocked(name) { bail!("name not allowed"); }
        }
        LedgerEntry::BirdCardClaim { bird_id, signed_by, .. } => {
            let Some(n) = name_of(chain, bird_id) else { bail!("bird has no name yet") };
            if n["named_by"].as_str() != Some(signed_by.as_str()) { bail!("only the namer can claim the card"); }
            if chain.store.state_get(&format!("bird_card_of:{bird_id}")).is_some() { bail!("card already minted"); }
        }
        LedgerEntry::BirdCardSend { card: c, to, signed_by, .. } => {
            let Some(cd) = card(chain, c) else { bail!("unknown card '{}'", c) };
            if cd["owner"].as_str() != Some(signed_by.as_str()) { bail!("only the card owner can send it"); }
            if to == signed_by { bail!("cannot send to yourself"); }
            if chain.store.get_account(to)?.is_none() { bail!("unknown account '{}'", to); }
        }
        LedgerEntry::BirdMerge { seat, from, into, evidence, epoch, .. } => {
            let pk = seat_pk(seat)?;
            pair_ok(chain, from, into, &pk)?;
            let seat_owner = owner_now(chain, seat, &pk);
            match evidence.as_str() {
                "auto" => {}
                "owner" => {
                    let (nf, ni) = (namer(chain, from), namer(chain, into));
                    let ok = (nf.is_none() && ni.is_none()) || nf.as_deref() == Some(seat_owner.as_str()) || ni.as_deref() == Some(seat_owner.as_str());
                    if !ok { bail!("owner evidence needs the seat's owner to have named one of the birds, or neither named"); }
                }
                _ => bail!("evidence must be auto or owner"),
            }
            if read_u64(chain, &rate_key(&pk, *epoch)) >= MAX_MERGES_PER_WINDOW {
                bail!("at most {} merges per seat per {} epochs", MAX_MERGES_PER_WINDOW, MERGE_WINDOW_EPOCHS);
            }
            if let Some(p) = get_json(chain, &pending_key(from, into)) {
                if p["filer_owner"].as_str() == Some(seat_owner.as_str()) { bail!("merge already pending from this account"); }
            }
        }
        LedgerEntry::BirdMergeConsent { from, into, signed_by, .. } => {
            let Some(p) = get_json(chain, &pending_key(from, into)) else { bail!("no pending merge {} -> {}", from, into) };
            if p["consenter"].as_str() != Some(signed_by.as_str()) { bail!("only '{}' can consent", p["consenter"].as_str().unwrap_or("")); }
        }
        LedgerEntry::BirdSplit { seat, bird_id, .. } => {
            let pk = seat_pk(seat)?;
            let Some(m) = get_json(chain, &format!("bird_merge:{bird_id}")) else { bail!("'{}' is not merged", bird_id) };
            let into = m["into"].as_str().unwrap_or_default();
            if !seen(chain, bird_id, &pk) || !seen(chain, into, &pk) { bail!("the seat must have reported both ids"); }
        }
        _ => bail!("not a bird entry"),
    }
    Ok(())
}
pub fn apply(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    check(chain, entry)?;
    match entry {
        LedgerEntry::BirdObserve { seat, bird_id, species, cell_res7, first_night, epoch, .. } => {
            let k = format!("bird_first:{bird_id}");
            if chain.store.state_get(&k).is_none() {
                let (_, pk) = hone_types::parse_device_clock_id(seat).unwrap_or_default();
                set_json(chain, &k, &json!({ "bird_id": bird_id, "seat": seat, "seat_pubkey": pk, "species": species,
                    "cell_res7": cell_res7, "first_night": first_night, "epoch": epoch }))?;
            }
            // Every seat (by key) that reported this bird: merge/split authority (rule 2).
            let (_, pk) = hone_types::parse_device_clock_id(seat).unwrap_or_default();
            chain.store.state_set(&format!("bird_seen:{bird_id}:{pk}"), epoch.to_string().as_bytes())?;
        }
        LedgerEntry::BirdName { bird_id, name, epoch, signed_by, .. } => {
            set_json(chain, &format!("bird_name:{bird_id}"), &json!({ "name": name, "named_by": signed_by, "epoch": epoch }))?;
            if let Some(nk) = name_key_for(chain, bird_id, name) { chain.store.state_set(&nk, bird_id.as_bytes())?; }
        }
        LedgerEntry::BirdCardClaim { bird_id, epoch, signed_by, .. } => {
            let f = first(chain, bird_id).unwrap_or_default();
            let n = name_of(chain, bird_id).unwrap_or_default();
            let id = card_id(bird_id);
            set_json(chain, &format!("bird_card:{id}"), &json!({ "card": id, "bird_id": bird_id, "species": f["species"],
                "cell_res7": f["cell_res7"], "cell_res6": f["cell_res7"].as_str().and_then(res6), "name": n["name"], "named_by": n["named_by"], "first_night": f["first_night"],
                "owner": signed_by, "merged_into": null,
                "history": [{ "epoch": epoch, "event": "mint", "to": signed_by }] }))?;
            chain.store.state_set(&format!("bird_card_of:{bird_id}"), id.as_bytes())?;
            chain.store.state_set(&format!("bird_card_owner:{signed_by}:{id}"), b"1")?;
        }
        LedgerEntry::BirdCardSend { card: c, to, epoch, signed_by, .. } => {
            let mut cd = card(chain, c).unwrap_or_default();
            if let Some(h) = cd["history"].as_array_mut() { h.push(json!({ "epoch": epoch, "event": "send", "from": signed_by, "to": to })); }
            cd["owner"] = json!(to);
            set_json(chain, &format!("bird_card:{c}"), &cd)?;
            chain.store.state_delete(&format!("bird_card_owner:{signed_by}:{c}"))?;
            chain.store.state_set(&format!("bird_card_owner:{to}:{c}"), b"1")?;
        }
        LedgerEntry::BirdMerge { seat, from, into, evidence, stats, epoch, .. } => {
            let pk = seat_pk(seat)?;
            let rk = rate_key(&pk, *epoch);
            chain.store.state_set(&rk, (read_u64(chain, &rk) + 1).to_string().as_bytes())?;
            let seat_owner = owner_now(chain, seat, &pk);
            let pk_key = pending_key(from, into);
            if let Some(mut p) = get_json(chain, &pk_key) {
                // A second backed seat of another account confirms the same merge (rule 4b).
                p["consent"] = json!({ "by_seat": seat, "epoch": epoch });
                set_json(chain, &pk_key, &p)?;
                return Ok(());
            }
            match (namer(chain, from), namer(chain, into)) {
                (Some(a), Some(b)) if a != b => {
                    // Named by different accounts: pending; lands at `due` only with consent.
                    let consenter = if seat_owner == a { b } else if seat_owner == b { a } else { later_namer(chain, from, into) };
                    set_json(chain, &pk_key, &json!({ "from": from, "into": into, "seat": seat, "seat_pubkey": pk,
                        "filer_owner": seat_owner, "consenter": consenter, "evidence": evidence, "stats": stats,
                        "epoch": epoch, "due": epoch + MERGE_CONSENT_EPOCHS, "consent": null }))?;
                }
                _ => merge(chain, from, into, *epoch, &pk)?,
            }
        }
        LedgerEntry::BirdMergeConsent { from, into, epoch, signed_by, .. } => {
            let k = pending_key(from, into);
            let mut p = get_json(chain, &k).unwrap_or_default();
            p["consent"] = json!({ "by": signed_by, "epoch": epoch });
            set_json(chain, &k, &p)?;
        }
        LedgerEntry::BirdSplit { seat, bird_id, epoch, .. } => {
            split(chain, bird_id, *epoch, &seat_pk(seat)?)?;
        }
        _ => {}
    }
    Ok(())
}

fn read_u64(chain: &Chain, k: &str) -> u64 {
    chain.store.state_get(k).and_then(|b| String::from_utf8(b).ok()).and_then(|s| s.parse().ok()).unwrap_or(0)
}
fn seat_pk(seat: &str) -> Result<String> {
    hone_types::parse_device_clock_id(seat).map(|(_, pk)| pk.to_owned()).ok_or_else(|| anyhow::anyhow!("bad seat id"))
}
fn owner_now(chain: &Chain, seat: &str, pk: &str) -> String {
    crate::usefulness::owner_of(&crate::sensor_seat::seat_for_pubkey(chain, pk).unwrap_or_else(|| seat.to_owned()))
}
fn seen(chain: &Chain, bird: &str, pk: &str) -> bool { chain.store.state_get(&format!("bird_seen:{bird}:{pk}")).is_some() }
fn namer(chain: &Chain, bird: &str) -> Option<String> { name_of(chain, bird).and_then(|n| n["named_by"].as_str().map(str::to_owned)) }
fn later_namer(chain: &Chain, a: &str, b: &str) -> String {
    let (na, nb) = (name_of(chain, a).unwrap_or_default(), name_of(chain, b).unwrap_or_default());
    let later = if na["epoch"].as_u64() < nb["epoch"].as_u64() { nb } else { na };
    later["named_by"].as_str().unwrap_or_default().to_owned()
}
fn rate_key(pk: &str, epoch: u64) -> String { format!("bird_merge_rate:{pk}:{}", epoch / MERGE_WINDOW_EPOCHS) }
fn pending_key(from: &str, into: &str) -> String { format!("bird_merge_pending:{from}:{into}") }

/// Rules 1, 2 and 7: both known and unmerged, same species and res-7 cell, species may be
/// named, and the seat (by key) reported both.
fn pair_ok(chain: &Chain, from: &str, into: &str, pk: &str) -> Result<()> {
    if from == into { bail!("cannot merge a bird into itself"); }
    let (Some(f), Some(i)) = (first(chain, from), first(chain, into)) else { bail!("unknown bird") };
    for b in [from, into] {
        if chain.store.state_get(&format!("bird_merged:{b}")).is_some() { bail!("'{}' is already merged", b); }
    }
    if f["species"] != i["species"] { bail!("different species"); }
    if f["cell_res7"] != i["cell_res7"] { bail!("different res-7 cell (no cross-cell merges)"); }
    if !naming_allowed(f["species"].as_str().unwrap_or_default()) { bail!("merging is not allowed for this species"); }
    if !seen(chain, from, pk) || !seen(chain, into, pk) { bail!("the seat must have reported both ids"); }
    Ok(())
}

/// Per quorum seal: pending merges whose window has passed land if consent arrived,
/// otherwise (or if the pair is no longer mergeable) they lapse.
pub fn tick(chain: &Chain, epoch: u64) {
    if epoch < activation_epoch() { return; }
    for (k, v) in chain.store.state_scan_prefix("bird_merge_pending:") {
        let Ok(p) = serde_json::from_slice::<Value>(&v) else { continue };
        if p["due"].as_u64().unwrap_or(0) > epoch { continue; }
        let (from, into) = (p["from"].as_str().unwrap_or_default(), p["into"].as_str().unwrap_or_default());
        let mergeable = from != into && [from, into].iter().all(|b| chain.store.state_get(&format!("bird_merged:{b}")).is_none());
        if !p["consent"].is_null() && mergeable {
            let _ = merge(chain, from, into, epoch, p["seat_pubkey"].as_str().unwrap_or_default());
        } else {
            let _ = set_json(chain, &format!("bird_merge_lapsed:{from}:{into}"), &json!({ "epoch": epoch }));
        }
        let _ = chain.store.state_delete(&k);
    }
}

/// Merge `from` into `into`. The earliest name wins and is kept on `into`; both cards stay,
/// `from`'s card points at `into` via merged_into. The previous `into` name is kept in the
/// merge record so a split can restore it. `by_seat_pk` is the merging seat's key.
pub fn merge(chain: &Chain, from: &str, into: &str, epoch: u64, by_seat_pk: &str) -> Result<()> {
    if from == into { bail!("cannot merge a bird into itself"); }
    let nf = name_of(chain, from);
    let ni = name_of(chain, into);
    let winner = match (nf, ni.clone()) {
        (Some(a), Some(b)) => Some(if a["epoch"].as_u64() < b["epoch"].as_u64() { a } else { b }),
        (a, b) => a.or(b),
    };
    if let Some(w) = &winner { set_json(chain, &format!("bird_name:{into}"), w)?; }
    chain.store.state_set(&format!("bird_merged:{from}"), into.as_bytes())?;
    set_json(chain, &format!("bird_merge:{from}"), &json!({ "into": into, "epoch": epoch, "by_seat": by_seat_pk, "prev_into_name": ni }))?;
    let into_card = chain.store.state_get(&format!("bird_card_of:{into}")).and_then(|b| String::from_utf8(b).ok());
    if let Some(fc) = chain.store.state_get(&format!("bird_card_of:{from}")).and_then(|b| String::from_utf8(b).ok()) {
        if let Some(mut cd) = card(chain, &fc) {
            cd["merged_into"] = json!({ "bird_id": into, "card": into_card, "epoch": epoch });
            if let Some(h) = cd["history"].as_array_mut() { h.push(json!({ "epoch": epoch, "event": "merged", "into": into })); }
            set_json(chain, &format!("bird_card:{fc}"), &cd)?;
        }
    }
    if let (Some(ic), Some(w)) = (into_card, winner) {
        if let Some(mut cd) = card(chain, &ic) {
            cd["name"] = w["name"].clone(); cd["named_by"] = w["named_by"].clone();
            set_json(chain, &format!("bird_card:{ic}"), &cd)?;
        }
    }
    Ok(())
}

/// Reverse a merge (rule 6): `from` is re-opened, `into` gets its pre-merge name back, the
/// absorbed card loses its merged_into pointer. Only pointers are removed. The merging seat
/// gets a reversed-merge count (input for slashing; not wired).
pub fn split(chain: &Chain, from: &str, epoch: u64, by_seat_pk: &str) -> Result<()> {
    let Some(m) = get_json(chain, &format!("bird_merge:{from}")) else { bail!("'{}' is not merged", from) };
    let into = m["into"].as_str().unwrap_or_default().to_owned();
    chain.store.state_delete(&format!("bird_merged:{from}"))?;
    chain.store.state_delete(&format!("bird_merge:{from}"))?;
    let prev = m["prev_into_name"].clone();
    if prev.is_null() { chain.store.state_delete(&format!("bird_name:{into}"))?; }
    else { set_json(chain, &format!("bird_name:{into}"), &prev)?; }
    if let Some(ic) = chain.store.state_get(&format!("bird_card_of:{into}")).and_then(|b| String::from_utf8(b).ok()) {
        if let Some(mut cd) = card(chain, &ic) {
            cd["name"] = prev["name"].clone(); cd["named_by"] = prev["named_by"].clone();
            set_json(chain, &format!("bird_card:{ic}"), &cd)?;
        }
    }
    if let Some(fc) = chain.store.state_get(&format!("bird_card_of:{from}")).and_then(|b| String::from_utf8(b).ok()) {
        if let Some(mut cd) = card(chain, &fc) {
            cd["merged_into"] = Value::Null;
            if let Some(h) = cd["history"].as_array_mut() { h.push(json!({ "epoch": epoch, "event": "split", "from": into, "by_seat": by_seat_pk })); }
            set_json(chain, &format!("bird_card:{fc}"), &cd)?;
        }
    }
    if let Some(s) = m["by_seat"].as_str() {
        let k = format!("bird_merge_reversed:{s}");
        chain.store.state_set(&k, (read_u64(chain, &k) + 1).to_string().as_bytes())?;
    }
    Ok(())
}

/// GET /api/bird/:bird_id
pub fn bird_view(chain: &Chain, bird: &str) -> Value {
    let c = chain.store.state_get(&format!("bird_card_of:{bird}")).and_then(|b| String::from_utf8(b).ok());
    json!({ "bird_id": bird, "first": first(chain, bird), "name": name_of(chain, bird),
        "merged_into": chain.store.state_get(&format!("bird_merged:{bird}")).and_then(|b| String::from_utf8(b).ok()),
        "card": c, "active_from_epoch": activation_epoch() })
}
/// GET /api/bird/card/:card
pub fn card_view(chain: &Chain, c: &str) -> Value { card(chain, c).unwrap_or(json!({ "card": c, "exists": false })) }
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor_seat::tests::*;
    use crate::tx::validate_and_apply;

    const E: u64 = 1_000;
    const B1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const B2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn observe(c: &Chain, seat: &str, bird: &str) -> anyhow::Result<String> {
        let e = LedgerEntry::BirdObserve { seat: seat.into(), bird_id: bird.into(), species: "Athene noctua".into(),
            cell_res7: "876d75d96ffffff".into(), first_night: "2026-11-04".into(), epoch: E, signed_by: seat.into() };
        validate_and_apply(c, &e, Some(&sig(&key(SEAT_SEED), &e)))
    }
    fn name(c: &Chain, who: &str, seed: u8, bird: &str, n: &str) -> anyhow::Result<String> {
        let e = LedgerEntry::BirdName { bird_id: bird.into(), name: n.into(), nonce: nonce(c, who), epoch: E, signed_by: who.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed), &e)))
    }
    fn claim(c: &Chain, who: &str, seed: u8, bird: &str) -> anyhow::Result<String> {
        let e = LedgerEntry::BirdCardClaim { bird_id: bird.into(), nonce: nonce(c, who), epoch: E, signed_by: who.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed), &e)))
    }
    fn send(c: &Chain, who: &str, seed: u8, card: &str, to: &str) -> anyhow::Result<String> {
        let e = LedgerEntry::BirdCardSend { card: card.into(), to: to.into(), nonce: nonce(c, who), epoch: E, signed_by: who.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed + 1), &e)))
    }
    fn backed_seat(c: &Chain) -> String {
        register(c, "josh", 0x10, SEAT_SEED).unwrap();
        stake(c, &seat_of("josh"), "josh", 0x10, 20 * 10_000_000_000).unwrap();
        seat_of("josh")
    }

    #[test]
    fn gated_and_names_validated() {
        let (c, _d) = world();
        let seat = backed_seat(&c);
        TEST_EPOCH.with(|x| x.set(Some(E + 1)));
        assert!(observe(&c, &seat, B1).is_err());
        TEST_EPOCH.with(|x| x.set(Some(E)));
        observe(&c, &seat, B1).unwrap();
        for bad in ["", " Lead", "Way too long a name for a bird", "R2D2", "Mr.Bird", "shithead", "a  b"] {
            assert!(name(&c, "josh", 0x10, B1, bad).is_err(), "{bad:?}");
        }
        assert!(valid_name("O'Neil-Ruiz") && valid_name("Señor Pico"));
    }

    #[test]
    fn first_namer_wins_and_non_owner_refused() {
        let (c, _d) = world();
        let seat = backed_seat(&c);
        observe(&c, &seat, B1).unwrap();
        assert!(name(&c, "mallory", 0x30, B1, "Mallory").is_err(), "not the first seat's owner");
        name(&c, "josh", 0x10, B1, "Pico").unwrap();
        assert!(name(&c, "josh", 0x10, B1, "Other").is_err(), "first name wins");
        assert_eq!(name_of(&c, B1).unwrap()["named_by"], "josh", "named_by is the account");
        assert!(name(&c, "josh", 0x10, B2, "Ghost").is_err(), "unknown bird");
    }

    #[test]
    fn claim_mints_once_and_send_moves_ownership_with_history() {
        let (c, _d) = world();
        let seat = backed_seat(&c);
        observe(&c, &seat, B1).unwrap();
        assert!(claim(&c, "josh", 0x10, B1).is_err(), "no name yet");
        name(&c, "josh", 0x10, B1, "Pico").unwrap();
        assert!(claim(&c, "mallory", 0x30, B1).is_err(), "only the namer");
        claim(&c, "josh", 0x10, B1).unwrap();
        assert!(claim(&c, "josh", 0x10, B1).is_err(), "unique");
        let id = card_id(B1);
        let cd = card(&c, &id).unwrap();
        assert_eq!((cd["owner"].as_str(), cd["name"].as_str(), cd["species"].as_str()), (Some("josh"), Some("Pico"), Some("Athene noctua")));
        assert!(send(&c, "mallory", 0x30, &id, "joe").is_err(), "not the owner");
        send(&c, "josh", 0x10, &id, "joe").unwrap();
        let cd = card(&c, &id).unwrap();
        assert_eq!(cd["owner"], "joe");
        assert_eq!(cd["history"].as_array().unwrap().len(), 2);
        assert_eq!(cd["named_by"], "josh", "namer stays");
        // Conservation: one card, exactly one owner index row.
        assert_eq!(c.store.state_scan_prefix("bird_card:").len(), 1);
        assert_eq!(c.store.state_scan_prefix("bird_card_owner:").len(), 1);
        assert!(c.store.state_get(&format!("bird_card_owner:joe:{id}")).is_some());
    }

    #[test]
    fn merge_keeps_earliest_name_and_both_cards() {
        let (c, _d) = world();
        let seat = backed_seat(&c);
        observe(&c, &seat, B1).unwrap();
        observe(&c, &seat, B2).unwrap();
        name(&c, "josh", 0x10, B1, "First").unwrap();
        claim(&c, "josh", 0x10, B1).unwrap();
        c.store.state_set(&format!("bird_name:{B2}"), br#"{"name":"Later","named_by":"josh","epoch":5000}"#).unwrap();
        claim(&c, "josh", 0x10, B2).unwrap();
        merge(&c, B1, B2, E + 10, "x").unwrap();
        assert_eq!(name_of(&c, B2).unwrap()["name"], "First", "earliest name wins");
        assert_eq!(card(&c, &card_id(B1)).unwrap()["merged_into"]["bird_id"], B2);
        assert_eq!(card(&c, &card_id(B2)).unwrap()["name"], "First");
        assert_eq!(c.store.state_scan_prefix("bird_card:").len(), 2, "both cards stay");
        assert!(name(&c, "josh", 0x10, B1, "Again").is_err(), "merged id cannot be renamed");
    }
}
#[cfg(test)]
mod merge_tests {
    use super::*;
    use crate::sensor_seat::tests::*;
    use crate::tx::validate_and_apply;
    use std::collections::BTreeMap;

    const E: u64 = 1_000;
    const H: u64 = 10_000_000_000;
    const B1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const B2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const B3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const OWL: &str = "Athene noctua";
    const CELL: &str = "876d75d96ffffff";
    const M_SEED: u8 = 0x5b; // mallory's seat key

    fn observe(c: &Chain, seat: &str, seed: u8, bird: &str, sp: &str, cell: &str) {
        let e = LedgerEntry::BirdObserve { seat: seat.into(), bird_id: bird.into(), species: sp.into(),
            cell_res7: cell.into(), first_night: "2026-11-04".into(), epoch: E, signed_by: seat.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed), &e))).unwrap();
    }
    fn mseat() -> String { format!("mallory@{}", pk(M_SEED)) }
    /// josh's and mallory's seats, both backed; josh's seat heard B1, B2, B3 (owls, one cell).
    fn setup() -> (Chain, tempfile::TempDir) {
        let (c, d) = world();
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
        stake(&c, &seat_of("josh"), "josh", 0x10, 20 * H).unwrap();
        register(&c, "mallory", 0x30, M_SEED).unwrap();
        stake(&c, &mseat(), "mallory", 0x30, 20 * H).unwrap();
        for b in [B1, B2, B3] { observe(&c, &seat_of("josh"), SEAT_SEED, b, OWL, CELL); }
        (c, d)
    }
    fn merge_e(c: &Chain, seat: &str, seed: u8, from: &str, into: &str, ev: &str, epoch: u64) -> anyhow::Result<String> {
        let stats: BTreeMap<String, u64> = [("nights_overlap".to_string(), 3), ("centroid_cos_q8".to_string(), 200)].into_iter().collect();
        let e = LedgerEntry::BirdMerge { seat: seat.into(), from: from.into(), into: into.into(), evidence: ev.into(),
            stats, epoch, signed_by: seat.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed), &e)))
    }
    fn split_e(c: &Chain, seat: &str, seed: u8, bird: &str, epoch: u64) -> anyhow::Result<String> {
        let e = LedgerEntry::BirdSplit { seat: seat.into(), bird_id: bird.into(), stats: BTreeMap::new(), epoch, signed_by: seat.into() };
        validate_and_apply(c, &e, Some(&sig(&key(seed), &e)))
    }
    fn set_name(c: &Chain, bird: &str, n: &str, by: &str, epoch: u64) {
        c.store.state_set(&format!("bird_name:{bird}"), serde_json::to_vec(&json!({ "name": n, "named_by": by, "epoch": epoch })).unwrap().as_slice()).unwrap();
    }
    fn merged(c: &Chain, b: &str) -> Option<String> {
        c.store.state_get(&format!("bird_merged:{b}")).map(|v| String::from_utf8(v).unwrap())
    }

    #[test]
    fn same_species_same_cell_seen_both_and_naming_gate() {
        let (c, _d) = setup();
        let j = seat_of("josh");
        let b4 = "4444444444444444444444444444444444444444444444444444444444444444";
        let b5 = "5555555555555555555555555555555555555555555555555555555555555555";
        let b6 = "6666666666666666666666666666666666666666666666666666666666666666";
        let b7 = "7777777777777777777777777777777777777777777777777777777777777777";
        observe(&c, &j, SEAT_SEED, b4, "Turdus grayi", CELL);
        observe(&c, &j, SEAT_SEED, b5, OWL, "876d75d97ffffff");
        observe(&c, &j, SEAT_SEED, b6, "Turdus grayi", CELL);
        observe(&c, &mseat(), M_SEED, b7, OWL, CELL);
        assert!(merge_e(&c, &j, SEAT_SEED, B1, b4, "auto", E).is_err(), "different species");
        assert!(merge_e(&c, &j, SEAT_SEED, B1, b5, "auto", E).is_err(), "different res-7 cell");
        assert!(merge_e(&c, &j, SEAT_SEED, b4, b6, "auto", E).is_err(), "species without naming_allowed");
        assert!(merge_e(&c, &j, SEAT_SEED, B1, b7, "auto", E).is_err(), "seat never reported b7");
        assert!(merge_e(&c, &j, SEAT_SEED, B1, B1, "auto", E).is_err(), "self merge");
        assert!(merge_e(&c, &j, SEAT_SEED, B1, B2, "guess", E).is_err(), "evidence kind");
        assert!(merge_e(&c, &j, 0x77, B1, B2, "auto", E).is_err(), "wrong seat key");
        // Naming is gated the same way.
        c.store.state_delete(&format!("bird_name:{b4}")).ok();
        let e = LedgerEntry::BirdName { bird_id: b4.into(), name: "Robin".into(), nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into() };
        assert!(validate_and_apply(&c, &e, Some(&sig(&key(0x10), &e))).is_err(), "naming refused for this species");
        // Unnamed pair, auto evidence: lands at once.
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E).unwrap();
        assert_eq!(merged(&c, B1).as_deref(), Some(B2));
        assert!(merge_e(&c, &j, SEAT_SEED, B1, B3, "auto", E + 1).is_err(), "already merged");
    }

    #[test]
    fn owner_evidence_only_for_own_or_unnamed_birds() {
        let (c, _d) = setup();
        let j = seat_of("josh");
        set_name(&c, B1, "Mal", "mallory", 10);
        assert!(merge_e(&c, &j, SEAT_SEED, B1, B2, "owner", E).is_err(), "josh named neither, one is named");
        set_name(&c, B2, "Jo", "josh", 20);
        // Owner evidence is now allowed (josh named B2); different namers -> pending, not merged.
        merge_e(&c, &j, SEAT_SEED, B1, B2, "owner", E).unwrap();
        assert!(merged(&c, B1).is_none());
        assert_eq!(get_json(&c, &pending_key(B1, B2)).unwrap()["consenter"], "mallory");
        // Same namer on both: owner merge of josh's own birds lands at once.
        set_name(&c, B3, "Jo2", "josh", 30);
        merge_e(&c, &j, SEAT_SEED, B3, B2, "owner", E + 1).unwrap();
        assert_eq!(merged(&c, B3).as_deref(), Some(B2));
    }

    #[test]
    fn different_namers_need_consent_and_the_window() {
        let (c, _d) = setup();
        let j = seat_of("josh");
        set_name(&c, B1, "Mal", "mallory", 10);
        set_name(&c, B2, "Jo", "josh", 20);
        // No consent: lapses at the due epoch.
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E).unwrap();
        assert!(merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E + 1).is_err(), "already pending from this account");
        tick(&c, E + MERGE_CONSENT_EPOCHS);
        assert!(merged(&c, B1).is_none());
        assert!(get_json(&c, &format!("bird_merge_lapsed:{B1}:{B2}")).is_some());
        // With consent from the other namer: lands only once the window has passed.
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E + 2).unwrap();
        let bad = LedgerEntry::BirdMergeConsent { from: B1.into(), into: B2.into(), nonce: nonce(&c, "joe"), epoch: E, signed_by: "joe".into() };
        assert!(validate_and_apply(&c, &bad, Some(&sig(&key(0x20), &bad))).is_err(), "only the other namer");
        let ok = LedgerEntry::BirdMergeConsent { from: B1.into(), into: B2.into(), nonce: nonce(&c, "mallory"), epoch: E, signed_by: "mallory".into() };
        validate_and_apply(&c, &ok, Some(&sig(&key(0x30), &ok))).unwrap();
        tick(&c, E + 2 + MERGE_CONSENT_EPOCHS - 1);
        assert!(merged(&c, B1).is_none(), "still inside the window");
        tick(&c, E + 2 + MERGE_CONSENT_EPOCHS);
        assert_eq!(merged(&c, B1).as_deref(), Some(B2));
        assert_eq!(name_of(&c, B2).unwrap()["name"], "Mal", "earliest name wins");
    }

    #[test]
    fn second_seat_of_another_account_confirms() {
        let (c, _d) = setup();
        let (j, m) = (seat_of("josh"), mseat());
        for b in [B1, B2] { observe(&c, &m, M_SEED, b, OWL, CELL); }
        set_name(&c, B1, "Mal", "mallory", 10);
        set_name(&c, B2, "Jo", "josh", 20);
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E).unwrap();
        merge_e(&c, &m, M_SEED, B1, B2, "auto", E + 1).unwrap();
        assert!(!get_json(&c, &pending_key(B1, B2)).unwrap()["consent"].is_null());
        tick(&c, E + MERGE_CONSENT_EPOCHS);
        assert_eq!(merged(&c, B1).as_deref(), Some(B2));
    }

    #[test]
    fn at_most_four_merges_per_seat_per_window() {
        let (c, _d) = setup();
        let j = seat_of("josh");
        c.store.state_set(&rate_key(&pk(SEAT_SEED), E), b"4").unwrap();
        assert!(merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E).is_err());
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E + MERGE_WINDOW_EPOCHS).unwrap();
        assert_eq!(read_u64(&c, &rate_key(&pk(SEAT_SEED), E + MERGE_WINDOW_EPOCHS)), 1);
    }

    #[test]
    fn split_reopens_the_absorbed_card_and_restores_the_name() {
        let (c, _d) = setup();
        let j = seat_of("josh");
        // Named and carded by josh (same namer -> immediate merge).
        set_name(&c, B1, "First", "josh", 10);
        set_name(&c, B2, "Second", "josh", 20);
        for b in [B1, B2] {
            let e = LedgerEntry::BirdCardClaim { bird_id: b.into(), nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into() };
            validate_and_apply(&c, &e, Some(&sig(&key(0x10), &e))).unwrap();
        }
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E).unwrap();
        assert_eq!(card(&c, &card_id(B2)).unwrap()["name"], "First");
        assert!(split_e(&c, &mseat(), M_SEED, B1, E + 1).is_err(), "splitter must have reported both");
        assert!(split_e(&c, &j, SEAT_SEED, B2, E + 1).is_err(), "B2 is not an absorbed id");
        split_e(&c, &j, SEAT_SEED, B1, E + 1).unwrap();
        assert!(merged(&c, B1).is_none());
        assert_eq!(name_of(&c, B2).unwrap()["name"], "Second", "pre-merge name restored");
        assert_eq!(card(&c, &card_id(B2)).unwrap()["name"], "Second");
        let c1 = card(&c, &card_id(B1)).unwrap();
        assert!(c1["merged_into"].is_null());
        assert_eq!(c1["history"].as_array().unwrap().last().unwrap()["event"], "split");
        assert_eq!(read_u64(&c, &format!("bird_merge_reversed:{}", pk(SEAT_SEED))), 1);
        assert_eq!(c.store.state_scan_prefix("bird_card:").len(), 2, "cards never deleted");
        // It can be merged again after a split.
        merge_e(&c, &j, SEAT_SEED, B1, B2, "auto", E + 2).unwrap();
    }

    #[test]
    fn gated_before_activation() {
        let (c, _d) = setup();
        TEST_EPOCH.with(|x| x.set(Some(E + 1)));
        assert!(merge_e(&c, &seat_of("josh"), SEAT_SEED, B1, B2, "auto", E).is_err());
    }
}
#[cfg(test)]
mod name_unique_tests {
    use super::*;
    use crate::sensor_seat::tests::*;
    use crate::tx::validate_and_apply;

    const E: u64 = 1_000;
    const H: u64 = 10_000_000_000;
    const B1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const B2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const B3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const B4: &str = "4444444444444444444444444444444444444444444444444444444444444444";
    const OWL: &str = "Athene noctua";
    const CELL: &str = "876d75d96ffffff";

    fn observe(c: &Chain, bird: &str, cell: &str) {
        let s = seat_of("josh");
        let e = LedgerEntry::BirdObserve { seat: s.clone(), bird_id: bird.into(), species: OWL.into(),
            cell_res7: cell.into(), first_night: "2026-11-04".into(), epoch: E, signed_by: s };
        validate_and_apply(c, &e, Some(&sig(&key(SEAT_SEED), &e))).unwrap();
    }
    fn name(c: &Chain, bird: &str, n: &str) -> anyhow::Result<String> {
        let e = LedgerEntry::BirdName { bird_id: bird.into(), name: n.into(), nonce: nonce(c, "josh"), epoch: E, signed_by: "josh".into() };
        validate_and_apply(c, &e, Some(&sig(&key(0x10), &e)))
    }
    /// A res-7 cell whose res-6 parent differs from CELL's.
    fn far_cell() -> String {
        let c = h3o::CellIndex::try_from(u64::from_str_radix(CELL, 16).unwrap()).unwrap();
        let p6 = c.parent(h3o::Resolution::Six).unwrap();
        let other = p6.grid_disk::<Vec<_>>(2).into_iter().find(|n| *n != p6).unwrap();
        format!("{:x}", u64::from(other.center_child(h3o::Resolution::Seven).unwrap()))
    }

    #[test]
    fn normalisation_collides_case_accents_space_confusables() {
        assert_eq!(norm_name("Bób"), norm_name("BOB"));
        assert_eq!(norm_name("bob"), norm_name("BOB"));
        assert_eq!(norm_name("Señor Pico"), norm_name("SENOR PICO"));
        assert_eq!(norm_name("Вob"), norm_name("Bob"), "Cyrillic В is a confusable of Latin B");
        assert_ne!(norm_name("Bob"), norm_name("Rob"));
        let r6 = res6(CELL).unwrap();
        assert_ne!(name_key(OWL, &r6, "Bob"), name_key("Tyto alba", &r6, "Bob"), "different species: different key");
    }

    #[test]
    fn unique_per_species_and_res6_cell_and_permanent() {
        let (c, _d) = world();
        register(&c, "josh", 0x10, SEAT_SEED).unwrap();
        stake(&c, &seat_of("josh"), "josh", 0x10, 20 * H).unwrap();
        for b in [B1, B2, B3] { observe(&c, b, CELL); }
        let far = far_cell();
        assert_ne!(res6(&far), res6(CELL));
        observe(&c, B4, &far);
        name(&c, B1, "Bob").unwrap();
        for dup in ["Bob", "BOB", "bob", "Bób"] { assert!(name(&c, B2, dup).is_err(), "{dup} collides in the same species and area"); }
        name(&c, B4, "Bob").unwrap();
        assert!(c.store.state_get(&name_key(OWL, &res6(CELL).unwrap(), "bob")).is_some());
        // Retired names stay taken: B1 merged into unnamed B3, then split; "Bob" is never freed.
        merge(&c, B1, B3, E, "x").unwrap();
        split(&c, B1, E + 1, "x").unwrap();
        assert!(name(&c, B2, "bob").is_err(), "a retired name stays taken");
        name(&c, B2, "Robin").unwrap();
        let e = LedgerEntry::BirdCardClaim { bird_id: B2.into(), nonce: nonce(&c, "josh"), epoch: E, signed_by: "josh".into() };
        validate_and_apply(&c, &e, Some(&sig(&key(0x10), &e))).unwrap();
        assert_eq!(card(&c, &card_id(B2)).unwrap()["cell_res6"], res6(CELL).unwrap(), "card stores the res-6 cell");
    }
}