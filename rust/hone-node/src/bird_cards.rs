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
        }
        LedgerEntry::BirdName { bird_id, name, epoch, signed_by, .. } => {
            set_json(chain, &format!("bird_name:{bird_id}"), &json!({ "name": name, "named_by": signed_by, "epoch": epoch }))?;
        }
        LedgerEntry::BirdCardClaim { bird_id, epoch, signed_by, .. } => {
            let f = first(chain, bird_id).unwrap_or_default();
            let n = name_of(chain, bird_id).unwrap_or_default();
            let id = card_id(bird_id);
            set_json(chain, &format!("bird_card:{id}"), &json!({ "card": id, "bird_id": bird_id, "species": f["species"],
                "cell_res7": f["cell_res7"], "name": n["name"], "named_by": n["named_by"], "first_night": f["first_night"],
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
        _ => {}
    }
    Ok(())
}

/// Merge `from` into `into` (the voiceprint pipeline decided they are one bird). The
/// earliest name wins and is kept on `into`; both cards stay, `from`'s card points at
/// `into` via merged_into. Deterministic; the authority that triggers it is the
/// voiceprint agent's merge entry (to be defined).
pub fn merge(chain: &Chain, from: &str, into: &str, epoch: u64) -> Result<()> {
    if from == into { bail!("cannot merge a bird into itself"); }
    let nf = name_of(chain, from);
    let ni = name_of(chain, into);
    let winner = match (nf, ni) {
        (Some(a), Some(b)) => Some(if a["epoch"].as_u64() < b["epoch"].as_u64() { a } else { b }),
        (a, b) => a.or(b),
    };
    if let Some(w) = &winner { set_json(chain, &format!("bird_name:{into}"), w)?; }
    chain.store.state_set(&format!("bird_merged:{from}"), into.as_bytes())?;
    let into_card = chain.store.state_get(&format!("bird_card_of:{into}")).and_then(|b| String::from_utf8(b).ok());
    if let Some(fc) = chain.store.state_get(&format!("bird_card_of:{from}")).and_then(|b| String::from_utf8(b).ok()) {
        if let Some(mut cd) = card(chain, &fc) {
            cd["merged_into"] = json!({ "bird_id": into, "card": into_card, "epoch": epoch });
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
        let e = LedgerEntry::BirdObserve { seat: seat.into(), bird_id: bird.into(), species: "Turdus grayi".into(),
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
        assert_eq!((cd["owner"].as_str(), cd["name"].as_str(), cd["species"].as_str()), (Some("josh"), Some("Pico"), Some("Turdus grayi")));
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
        merge(&c, B1, B2, E + 10).unwrap();
        assert_eq!(name_of(&c, B2).unwrap()["name"], "First", "earliest name wins");
        assert_eq!(card(&c, &card_id(B1)).unwrap()["merged_into"]["bird_id"], B2);
        assert_eq!(card(&c, &card_id(B2)).unwrap()["name"], "First");
        assert_eq!(c.store.state_scan_prefix("bird_card:").len(), 2, "both cards stay");
        assert!(name(&c, "josh", 0x10, B1, "Again").is_err(), "merged id cannot be renamed");
    }
}