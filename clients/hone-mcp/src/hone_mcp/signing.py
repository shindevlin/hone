"""Canonical signing-message builder for HONE ledger entries.

Ground truth: `canonical_signing_message()` in rust/hone-node/src/tx.rs of the
`shindevlin/hone` repo, and — critically — a LIVE two-node test against a
real compiled `hone-node` binary on 2026-10-25, which caught a wrong
assumption that a pure source read did not: see point 1 below.

Two independent facts combine to decide the wire format, and getting either
one wrong produces a signature the node silently rejects:

1. **The message is INSERTION order (exactly the field order written in each
   `json!({...})` macro call), NOT alphabetical.** The first version of this
   module assumed alphabetical order, reasoning that no Cargo.toml in the
   hone-node workspace requests serde_json's `preserve_order` feature
   directly — true, but irrelevant: Cargo unifies features across the whole
   dependency graph for one build, and the `nostr` crate (pulled in
   transitively, likely for relay/discovery) requests
   `"serde_json/preserve_order"` in ITS OWN Cargo.toml. That silently turns
   preserve_order on for every use of serde_json in the hone-node binary,
   including `canonical_signing_message`'s `json!()` calls — confirmed by
   compiling hone-node for real and printing the actual message for a known
   entry (`cargo test -p hone-node ... -- --nocapture`), which came back in
   source order, not sorted. A plausible-looking message built the wrong way
   fails signature verification with no diagnostic pointing at "order" — the
   live test is what caught it; reading the source alone did not. See
   concerns.md for why this is also a chain-repo risk, not just a client bug.

   One exception: a `keys` map (as in `AccountCreate`) is collected into a
   Rust `std::collections::BTreeMap` *before* being embedded in the `json!()`
   call — a plain Rust `BTreeMap`, unrelated to serde_json's `Value`
   representation — so that one sub-object's keys ARE alphabetically sorted,
   regardless of the preserve_order feature. `_account_create` below handles
   this explicitly; every other builder is plain insertion order.

2. Most — but not all — LedgerEntry variants are signed this way (their
   match arm in `canonical_signing_message` builds a `json!({...})` of a
   hand-picked subset of client-known fields, explicitly excluding
   server-assigned fields like `epoch`). This module implements ONLY that
   documented, explicit-arm path ("Format A") for the variants relevant to
   node onboarding/operation.

   A second group of variants (anything NOT given an explicit arm — notably
   `ClockNodeRegister`) falls through to a catch-all arm that signs the
   FULL entry (`{"chain_id":..., "entry": <typed-serialize of the entry,
   struct-declaration field order>}`). That "Format B" is deliberately NOT
   implemented here: it is used for mainnet/testnet replay protection on
   entries this server does not sign client-side, and the two places it
   would matter for onboarding (clock registration, testnet opt-in) already
   have zero-key-custody self-sign endpoints on the node itself
   (`/api/clock/self-register`, `/api/node/testnet/toggle`) — the node signs
   with its own already-loaded key, so this module is never needed for them.
   See concerns.md and README.md.

This server never holds, derives, or transmits a private key. These
functions only build the *message a human's own signer must sign* and the
*body to submit once signed* — signing itself happens outside this process.

IMPORTANT — which key role must sign: Transfer/Stake/Unstake are verified
against the account's **"active"** role key specifically (not "posting"),
per `check_signature(..., "active")` in `tx.rs`. Every other Format-A entry
here is verified against **"posting"**. Signing with the wrong role's key
doesn't error — it just silently matches `None` (no such role registered
yet) and the check is skipped, or fails to verify if that role IS
registered with a different key. `server.py`'s per-tool docs state the
correct role for each tx_type.
"""

from __future__ import annotations

import json
from typing import Any, Callable

# ── Format-A signing-field builders ───────────────────────────────────────
# Each returns the dict of fields to sign, EXCLUDING chain_id/type (added by
# build_signing_message). Keys may be in any order here — canonicalization
# (alphabetical sort, compact separators) happens once, centrally, below.

def _transfer(f: dict) -> dict:
    return {"from": f["from"], "to": f["to"], "amount": int(f["amount"]), "token": f["token"], "nonce": int(f["nonce"])}


def _stake(f: dict) -> dict:
    return {"account": f["account"], "amount": int(f["amount"]), "nonce": int(f["nonce"])}


def _unstake(f: dict) -> dict:
    return {"account": f["account"], "amount": int(f["amount"]), "nonce": int(f["nonce"])}


def _sensor_register(f: dict) -> dict:
    return {
        "sensor_id": f["sensor_id"],
        "owner": f["owner"],
        "sensor_type": f["sensor_type"],
        "location": f.get("location"),
        "signed_by": f["signed_by"],
    }


def _sensor_data_commit(f: dict) -> dict:
    return {
        "sensor_id": f["sensor_id"],
        "owner": f["owner"],
        "batch_hash": f["batch_hash"],
        "reading_count": int(f["reading_count"]),
        "sensor_type": f["sensor_type"],
        "signed_by": f["signed_by"],
    }


def _sensor_vouch(f: dict) -> dict:
    return {"sensor_id": f["sensor_id"], "voucher": f["voucher"], "signed_by": f["signed_by"]}


def _gateway_heartbeat(f: dict) -> dict:
    return {"gateway_id": f["gateway_id"], "owner": f["owner"], "signed_by": f["signed_by"]}


def _node_role_opt_in(f: dict) -> dict:
    return {
        "node": f["node"],
        "role": f["role"],
        "backer_share_bps": int(f["backer_share_bps"]),
        "max_backers": int(f["max_backers"]),
        "nonce": int(f["nonce"]),
    }


def _node_role_opt_out(f: dict) -> dict:
    return {"node": f["node"], "role": f["role"], "nonce": int(f["nonce"])}


def _node_role_stake(f: dict) -> dict:
    return {
        "node": f["node"],
        "role": f["role"],
        "staker": f["staker"],
        "amount": int(f["amount"]),
        "nonce": int(f["nonce"]),
    }


def _node_role_unstake(f: dict) -> dict:
    return {
        "node": f["node"],
        "role": f["role"],
        "staker": f["staker"],
        "amount": int(f["amount"]),
        "nonce": int(f["nonce"]),
    }


def _account_update_key(f: dict) -> dict:
    return {"account": f["account"], "role": f["role"], "new_public_key": f["new_public_key"]}


def _account_set_primary(f: dict) -> dict:
    return {"account": f["account"], "primary": f["primary"]}


def _account_api_key_set(f: dict) -> dict:
    return {"account": f["account"], "api_key": f["api_key"], "nonce": int(f["nonce"])}


def _account_create(f: dict) -> dict:
    keys = f.get("keys") or {}
    proofs = sorted(
        (f.get("chain_proofs") or []),
        key=lambda p: p.get("chain", ""),
    )
    canonical_proofs = [
        {"chain": p["chain"], "commitment": p["commitment"], "mode": p["mode"]} for p in proofs
    ]
    return {
        "account": f["account"],
        # Rust collects `keys` into a std::collections::BTreeMap before embedding
        # it — a plain Rust BTreeMap, always sorted by key, independent of
        # serde_json's preserve_order feature. Sort explicitly to match.
        "keys": dict(sorted(keys.items())),
        "chain_proofs": canonical_proofs,
        "funded_by": f.get("funded_by"),
    }


# tx_type (SCREAMING_SNAKE_CASE, matches the node's own `type` tag) -> builder
#
# NOTE: SensorKeyRegister and DeviceKeyRegister are real LedgerEntry variants
# (hone_types::entry::LedgerEntry) with their own canonical-signing-message arms,
# but — as of the 2026-10-25 read of rust/hone-node/src/api.rs — neither has an
# HTTP route wired up (no `.route("/api/sensor/key-register", ...)` or
# `.route("/api/device/key-register", ...)` exists, and there is no generic
# raw-entry submission endpoint either). They are deliberately left out of
# _BUILDERS so callers get a clear "not submittable" error instead of a
# plausible-looking message for an endpoint that 404s. See concerns.md.
_BUILDERS: dict[str, Callable[[dict], dict]] = {
    "TRANSFER": _transfer,
    "STAKE": _stake,
    "UNSTAKE": _unstake,
    "SENSOR_REGISTER": _sensor_register,
    "SENSOR_DATA_COMMIT": _sensor_data_commit,
    "SENSOR_VOUCH": _sensor_vouch,
    "GATEWAY_HEARTBEAT": _gateway_heartbeat,
    "NODE_ROLE_OPT_IN": _node_role_opt_in,
    "NODE_ROLE_OPT_OUT": _node_role_opt_out,
    "NODE_ROLE_STAKE": _node_role_stake,
    "NODE_ROLE_UNSTAKE": _node_role_unstake,
    "ACCOUNT_UPDATE_KEY": _account_update_key,
    "ACCOUNT_SET_PRIMARY": _account_set_primary,
    "ACCOUNT_API_KEY_SET": _account_api_key_set,
    "ACCOUNT_CREATE": _account_create,
}

SUPPORTED_TX_TYPES = sorted(_BUILDERS.keys())

# Entries known to exist on-chain but deliberately unsupported here because
# they use the full-entry ("Format B") signing arm. Surfaced in error
# messages so a caller isn't left guessing.
KNOWN_UNSUPPORTED_FORMAT_B_HINTS = {
    "CLOCK_NODE_REGISTER": (
        "ClockNodeRegister uses full-entry signing (Format B), not implemented here. "
        "Use the hone_clock_self_register tool instead — the node signs with its own "
        "already-configured key, no external signature needed."
    ),
    "TESTNET_OPERATOR_REGISTER": (
        "TestnetOperatorRegister uses full-entry signing (Format B), not implemented here. "
        "Use the hone_testnet_toggle tool instead — the node signs with its own "
        "already-configured key, no external signature needed."
    ),
    "SENSOR_KEY_REGISTER": (
        "SensorKeyRegister has no HTTP route on the node (checked rust/hone-node/src/api.rs "
        "on 2026-10-25) — it cannot be submitted via this API at all yet. See concerns.md."
    ),
    "DEVICE_KEY_REGISTER": (
        "DeviceKeyRegister has no HTTP route on the node (checked rust/hone-node/src/api.rs "
        "on 2026-10-25) — it cannot be submitted via this API at all yet. See concerns.md."
    ),
}


class UnsupportedTxType(ValueError):
    pass


def build_signing_message(tx_type: str, fields: dict[str, Any], chain_id: str) -> str:
    """Build the exact canonical message string a signer must sign.

    `tx_type` is the SCREAMING_SNAKE_CASE entry tag (e.g. "TRANSFER").
    `fields` carries the entry's user-controlled fields using the same
    names the HTTP API body uses (see README's endpoint table).
    `chain_id` must match the target node's actual chain_id exactly
    ("hone" for mainnet, "hone-testnet" for testnet, or a custom isolated
    chain_id) — the node has no GET endpoint that reports this value, so the
    caller must know and supply it. A wrong chain_id produces a signature
    that verifies fine cryptographically but is rejected by every node on
    the intended chain (and would be *accepted* by a node on the wrong
    chain — this is the chain's own replay-protection design, see
    concerns.md).
    """
    tx_type = tx_type.upper()
    builder = _BUILDERS.get(tx_type)
    if builder is None:
        hint = KNOWN_UNSUPPORTED_FORMAT_B_HINTS.get(tx_type)
        msg = f"unsupported tx_type '{tx_type}'. Supported: {', '.join(SUPPORTED_TX_TYPES)}."
        if hint:
            msg += f" {hint}"
        raise UnsupportedTxType(msg)

    signed_fields = builder(fields)
    message_obj = {"chain_id": chain_id, "type": tx_type, **signed_fields}
    # NO sort_keys: insertion order (see module docstring point 1) is what the
    # real compiled node produces, confirmed by a live test on 2026-10-25.
    # Python dicts preserve insertion order since 3.7, so building `message_obj`
    # in the right order (as every builder above does, one-to-one with each
    # Rust match arm's own field order) is sufficient.
    return json.dumps(message_obj, separators=(",", ":"), ensure_ascii=False)
