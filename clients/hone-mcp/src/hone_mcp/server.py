"""hone-mcp server: exposes a hone-node's local HTTP API as MCP tools.

Run with:  python -m hone_mcp.server   (stdio transport, for Claude Desktop etc.)

Configuration (environment variables):
    HONE_NODE_URL   — base URL of the node's local HTTP API (default: http://127.0.0.1:4242)
    HONE_CHAIN_ID   — "hone" (mainnet) or "hone-testnet" (testnet), or a custom isolated
                       chain_id. Required by the *_prepare_signing_message tool only — the
                       node's API has no endpoint that reports its own chain_id (see
                       concerns.md). Not needed for any other tool.

Security model (see README.md "Security model" for the full rationale):
    - This server NEVER accepts, stores, derives, or transmits a private key, mnemonic,
      or raw signature-producing secret as a tool argument or return value.
    - Reads are unrestricted (they're public chain data anyway).
    - Writes that need no signature (first key registration for a role with no key
      yet, per the node's own bootstrap-grace-period rules) are exposed directly.
      NOTE: brand-new account creation is NOT in this tier — it always requires a
      signature from the owner key being registered (live-verified against a real
      node; see the note above hone_create_account). It's handled by the tier-4
      flow below instead, with hone_create_account as its convenience wrapper.
    - Writes the NODE can self-sign with its own already-configured key
      (HONE_POSTING_KEY on the node) are exposed directly — zero custody risk, since the
      key never leaves the node process.
    - Writes that require a signature from an EXISTING keyed account are split into a
      `hone_prepare_signing_message` tool (returns the message to sign + the body to
      submit) and a `hone_submit_signed_entry` tool (submits a body that already has a
      `signature` filled in, obtained however the operator obtains it — e.g. their
      existing hone-transfer/hone-role-stake/hone-sensor-enroll CLI helpers, or any
      ed25519 signer over their own key). This server is never in the signing loop for
      those.
"""

from __future__ import annotations

import os
from typing import Any

from mcp.server.mcpserver import MCPServer

from .client import HoneApiError, HoneClient, hone_to_hunits
from .signing import SUPPORTED_TX_TYPES, UnsupportedTxType, build_signing_message

mcp = MCPServer(
    name="hone",
    title="HONE Node",
    version="0.1.0",
    instructions=(
        "Tools for installing, onboarding, and operating a HONE node via its local HTTP "
        "API. Start with hone_node_health to confirm the node is reachable, and "
        "hone_staking_requirements / hone_get_account to see what an operator needs "
        "before registering a role. hone_register_key and hone_clock_self_register "
        "need no external signature at all (zero key custody for this server) — they "
        "are the fastest path to a working install for an account that already exists. "
        "hone_create_account needs one signature from the new owner key being "
        "registered (see its own docstring); every other write needs a signature too, "
        "via hone_prepare_signing_message + hone_submit_signed_entry."
    ),
)


def _client() -> HoneClient:
    return HoneClient()


def _err(e: HoneApiError) -> dict[str, Any]:
    return {"ok": False, "error": str(e), "status_code": e.status_code, "detail": e.body}


# ── Read-only / status tools ──────────────────────────────────────────────────

@mcp.tool()
async def hone_node_health() -> dict[str, Any]:
    """Check whether the local hone-node is up and reachable. Call this first."""
    c = _client()
    try:
        health = await c.get("/health")
        latest = await c.get("/api/latest")
        return {"ok": True, "node_url": c.base_url, "health": health, "chain": latest}
    except HoneApiError as e:
        return {"ok": False, "node_url": c.base_url, "error": str(e)}


@mcp.tool()
async def hone_get_balance(account: str) -> dict[str, Any]:
    """Get an account's native HONE balance (and hunits, the integer base unit)."""
    try:
        return await _client().get(f"/api/balance/{account}")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_get_all_balances(account: str) -> dict[str, Any]:
    """Get every token balance (not just native HONE) held by an account."""
    try:
        return await _client().get(f"/api/balances/{account}")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_get_account(account: str) -> dict[str, Any]:
    """Get an account's registered keys, stake, nonce, and proven external chains.

    Returns 404 (as {"ok": false, "status_code": 404, ...}) if the account does not
    exist yet — that's the signal to use hone_create_account instead.
    """
    try:
        return await _client().get(f"/api/account/{account}")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_get_account_history(account: str, limit: int = 50) -> dict[str, Any]:
    """Get an account's recent on-chain transaction history (most recent first)."""
    try:
        return await _client().get(f"/api/account/{account}/history", params={"limit": limit})
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_get_stake(account: str) -> dict[str, Any]:
    """Get an account's general (non-role) stake amount."""
    try:
        return await _client().get(f"/api/stake/{account}")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_staking_requirements() -> dict[str, Any]:
    """Get the current minimum stake (in HONE and hunits) required for each node role
    (clock, miner, storage, sensor, service, verifier, mempool). Call this before
    registering a device for any role, so you know how much to stake.
    """
    try:
        return await _client().get("/api/staking/requirements")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_node_role_positions(node: str, role: str | None = None) -> dict[str, Any]:
    """Get a node's backer-stake positions for one role, or all roles if role is omitted."""
    try:
        path = f"/api/node/role/stakes/{node}/{role}" if role else f"/api/node/role/stakes/{node}"
        return await _client().get(path)
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_clock_status() -> dict[str, Any]:
    """Get this node's clock-consensus status: whether it's in quorum, registered
    clocks on the network, and peer count. Use this to check whether
    hone_clock_self_register worked.
    """
    try:
        return await _client().get("/api/clock/status")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_clock_uptime(node_id: str) -> dict[str, Any]:
    """Get a clock node's uptime stats and current reward multiplier."""
    try:
        return await _client().get(f"/api/clock/{node_id}/uptime")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_list_sensors() -> dict[str, Any]:
    """List all registered sensors on the network."""
    try:
        return await _client().get("/api/sensors")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_get_sensor(sensor_id: str) -> dict[str, Any]:
    """Get one sensor's registration record."""
    try:
        return await _client().get(f"/api/sensor/{sensor_id}")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_explorer_status() -> dict[str, Any]:
    """Get a network-wide snapshot: chain height, account count, circulating supply,
    and active role counts over the last ~100 epochs.
    """
    try:
        return await _client().get("/api/explorer/status")
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_faucet_status(account: str | None = None) -> dict[str, Any]:
    """Get testnet faucet availability and claim size (testnet only; 403s on mainnet)."""
    try:
        return await _client().get("/api/faucet/status", params={"account": account} if account else None)
    except HoneApiError as e:
        return _err(e)


# ── Bootstrap tools ────────────────────────────────────────────────────────
#
# NOTE: AccountCreate is NOT signature-free, despite being a first-ever
# registration. Live-tested against a real node on 2026-10-25: it failed with
# "account creation requires an owner public key" (unsigned, no keys), and
# again with "signature required for account creation" once an owner key was
# added. The node's `check_account_create_signature` unconditionally requires
# a valid ed25519 signature from the OWNER KEY BEING REGISTERED, to prove the
# caller holds that key's private half before it's trusted on-chain — the
# generic "new account, nothing to verify against yet" exemption that DOES
# apply to hone_register_key below does NOT apply here. This server still
# never sees that private key: hone_create_account takes an
# already-produced signature, obtained via hone_prepare_signing_message
# (tx_type="ACCOUNT_CREATE") + your own ed25519 signer.

@mcp.tool()
async def hone_create_account(
    account: str,
    owner_public_key_hex: str,
    signature_hex: str,
    posting_public_key_hex: str | None = None,
    active_public_key_hex: str | None = None,
    funded_by: str | None = None,
) -> dict[str, Any]:
    """Create a brand-new HONE account. Requires a signature from the owner key
    being registered (see the module-level note above for why — this is NOT
    optional, the node rejects an unsigned AccountCreate).

    Two-step flow:
    1. Generate an owner ed25519 keypair yourself (hone-cli, or any ed25519 tool
       — this server never generates or sees private keys). If you also want a
       posting and/or active key set at creation time (needed for most other
       actions — "active" specifically for Transfer/Stake/Unstake — see
       hone_prepare_signing_message's docs), generate those too. Call
       hone_prepare_signing_message(tx_type="ACCOUNT_CREATE", fields={"account":
       account, "keys": {"owner": owner_public_key_hex, "posting": ...,
       "active": ...}}) — include every key role you're passing here, in this
       same call, or the signature won't match what this tool submits — to get
       the exact message to sign.
    2. Sign that message with the new owner private key, hex-encode the 64-byte
       ed25519 signature, and pass it here as `signature_hex`.

    `funded_by` is the existing account that pays the NAME_REGISTRATION_STAKE if
    name-staking governance is currently enabled (it defaults to off network-wide,
    in which case `funded_by` isn't needed at all).
    """
    keys: dict[str, str] = {"owner": owner_public_key_hex}
    if posting_public_key_hex:
        keys["posting"] = posting_public_key_hex
    if active_public_key_hex:
        keys["active"] = active_public_key_hex
    body: dict[str, Any] = {"account": account, "keys": keys, "signature": signature_hex}
    if funded_by:
        body["funded_by"] = funded_by
    try:
        return await _client().post("/api/account/create", body)
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_register_key(account: str, role: str, new_public_key_hex: str) -> dict[str, Any]:
    """Register a public key for a role ("posting", "owner", "memo", "hide", "seek",
    "active") on an account. No signature is required ONLY while the account has
    NO owner key and NO posting key at all (the node's "grace period") — this is
    how a brand-new, keyless account gets its very first key or two.

    Live-tested on 2026-10-25: the grace period is account-wide, not per-role.
    AccountUpdateKey's own authorization check ignores which specific `role` is
    being set and always demands a signature from the owner key (falling back to
    posting if no owner key exists) once *any* key already exists on the account —
    so this is NOT "no signature needed if this particular role is still empty."
    Example: an account with only a posting key set can NOT silently add an
    "active" key through this tool; that add already requires a signature. Once
    an account has an owner or posting key at all, use
    hone_prepare_signing_message with tx_type="ACCOUNT_UPDATE_KEY" plus
    hone_submit_signed_entry instead (signed_fields need role="owner", falling
    back to "posting" only if no owner key exists).
    """
    try:
        return await _client().post(
            "/api/account/update-key",
            {"account": account, "role": role, "new_public_key": new_public_key_hex, "signed_by": account},
        )
    except HoneApiError as e:
        return _err(e)


# ── Node self-sign tools (zero key custody — the node signs with its own key) ─

@mcp.tool()
async def hone_clock_self_register(stake_hone: float | None = None) -> dict[str, Any]:
    """Register THIS node as a clock-consensus participant, using the node's own
    already-configured signing key (HONE_POSTING_KEY on the node). No external
    signature is needed or accepted — the node signs internally. This is the
    single call that turns a fresh install into a clock node, once the operator's
    account is staked.

    `stake_hone` is optional; defaults to the current clock_min_stake chain
    parameter (see hone_staking_requirements) if omitted.
    """
    body: dict[str, Any] = {}
    if stake_hone is not None:
        body["stake"] = hone_to_hunits(stake_hone)
    try:
        return await _client().post("/api/clock/self-register", body)
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_testnet_toggle(enabled: bool = True) -> dict[str, Any]:
    """Opt this node's account in (or out) of testnet-operator rewards, using the
    node's own already-configured signing key. No external signature needed.
    """
    try:
        return await _client().post("/api/node/testnet/toggle", {"enabled": enabled})
    except HoneApiError as e:
        return _err(e)


@mcp.tool()
async def hone_testnet_status(account: str) -> dict[str, Any]:
    """Check whether an account is a registered testnet operator."""
    try:
        return await _client().get(f"/api/node/testnet/status/{account}")
    except HoneApiError as e:
        return _err(e)


# ── Generic prepare/submit for entries that need an EXISTING account's signature ──
#
# Which role key must sign (verified live against a real node on 2026-10-25,
# via rust/hone-node/src/tx.rs's per-arm check_signature(..., role) calls):
#   "active"  -> TRANSFER, STAKE, UNSTAKE
#   "posting" -> every other tx_type in SUPPORTED_TX_TYPES
# Signing with the wrong role's key fails silently in one direction: if that
# role has no key registered at all yet, the check is skipped (not an error);
# if it DOES have a different key registered, the submission is rejected with
# a signature-verification error and no hint that "role" was the problem.
_SIGNING_ROLE_BY_TX_TYPE = {
    "TRANSFER": "active",
    "STAKE": "active",
    "UNSTAKE": "active",
}


@mcp.tool()
async def hone_prepare_signing_message(tx_type: str, fields: dict[str, Any], chain_id: str | None = None) -> dict[str, Any]:
    """Build the exact message an operator's own signer must produce an ed25519
    signature over, for a transaction type that needs an existing account's key
    (transfer, stake, node-role backing, sensor registration/commit, etc.) — plus the
    exact JSON body to submit afterwards via hone_submit_signed_entry.

    This server does not sign anything itself: it never accepts or holds a private
    key. Sign the returned `message_to_sign` string with your own ed25519 key (e.g.
    the operator's existing hone-transfer/hone-role-stake/hone-sensor-enroll CLI
    helpers, or any ed25519 signer), then call hone_submit_signed_entry with the
    resulting hex signature filled into the returned `submit_body`.

    IMPORTANT: sign with the right role's key — TRANSFER/STAKE/UNSTAKE need the
    account's "active" key specifically; every other tx_type needs "posting".
    The response's `signing_role_required` field states which one for the
    tx_type you asked for. Signing with the wrong role's key doesn't raise a
    clear error on submission — it's silently skipped (if that role has no key
    registered) or rejected as a plain verification failure (if it does).

    `tx_type` is one of: """ + ", ".join(SUPPORTED_TX_TYPES) + """.

    `fields` carries the entry's own fields by name (e.g. for TRANSFER:
    {"from", "to", "amount" (HONE, float), "token" (default "HONE"), "nonce",
    "memo"}). Fetch the account's current `nonce` first via hone_get_account and
    submit `nonce + 1` — the chain rejects any other value.

    `chain_id` must match the TARGET node's actual chain_id ("hone" for mainnet,
    "hone-testnet" for testnet). Defaults to the HONE_CHAIN_ID environment variable
    if not given. There is no API endpoint to look this up automatically (see
    concerns.md) — get it wrong and the signature silently fails verification on the
    intended chain.
    """
    resolved_chain_id = chain_id or os.environ.get("HONE_CHAIN_ID")
    if not resolved_chain_id:
        return {
            "ok": False,
            "error": (
                "chain_id not given and HONE_CHAIN_ID is not set. Pass chain_id "
                "explicitly (\"hone\" for mainnet, \"hone-testnet\" for testnet)."
            ),
        }

    # Amounts come in as human HONE floats in `fields`; convert the ones that are
    # amounts before building the message (hunits must be an exact integer).
    normalized = dict(fields)
    for amount_key in ("amount",):
        if amount_key in normalized and isinstance(normalized[amount_key], (int, float)):
            normalized[amount_key] = hone_to_hunits(float(normalized[amount_key]))

    try:
        message = build_signing_message(tx_type, normalized, resolved_chain_id)
    except UnsupportedTxType as e:
        return {"ok": False, "error": str(e)}
    except KeyError as e:
        return {"ok": False, "error": f"missing required field {e} for tx_type '{tx_type}'"}

    try:
        submit_endpoint, submit_body = _submit_shape(tx_type.upper(), normalized)
    except (KeyError, UnsupportedTxType) as e:
        return {"ok": False, "error": str(e)}
    signing_role = _SIGNING_ROLE_BY_TX_TYPE.get(tx_type.upper(), "posting")
    return {
        "ok": True,
        "message_to_sign": message,
        "chain_id_used": resolved_chain_id,
        "signing_role_required": signing_role,
        "submit_endpoint": submit_endpoint,
        "submit_body": submit_body,
        "note": f"Sign message_to_sign with the account's '{signing_role}' role key "
                "specifically (wrong role silently skips or fails verification — see "
                "this tool's docstring), hex-encode the 64-byte ed25519 signature, put "
                "it in submit_body.signature, then call "
                "hone_submit_signed_entry(endpoint=submit_endpoint, body=submit_body).",
    }


def _submit_shape(tx_type: str, f: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    """Map a tx_type + its signing fields to the (endpoint, body-with-blank-signature)
    the node's HTTP API actually expects. Mirrors each handler's `Body` struct in
    rust/hone-node/src/api.rs exactly (field names, not shapes invented here).
    """
    if tx_type == "TRANSFER":
        return "/api/transfer", {
            "from": f["from"], "to": f["to"], "amount": f["amount"], "token": f.get("token", "HONE"),
            "memo": f.get("memo"), "signed_by": f["from"], "nonce": f["nonce"], "signature": "",
        }
    if tx_type == "STAKE":
        return "/api/stake", {"account": f["account"], "amount": f["amount"], "nonce": f["nonce"], "signed_by": f["account"], "signature": ""}
    if tx_type == "UNSTAKE":
        return "/api/unstake", {"account": f["account"], "amount": f["amount"], "nonce": f["nonce"], "signed_by": f["account"], "signature": ""}
    if tx_type == "SENSOR_REGISTER":
        return "/api/sensor/register", {
            "sensor_id": f["sensor_id"], "owner": f["owner"], "sensor_type": f["sensor_type"],
            "location": f.get("location"), "metadata": f.get("metadata"), "signature": "",
        }
    if tx_type == "SENSOR_DATA_COMMIT":
        return "/api/sensor/commit", {
            "sensor_id": f["sensor_id"], "owner": f["owner"], "batch_hash": f["batch_hash"],
            "reading_count": f["reading_count"], "sensor_type": f["sensor_type"],
            "value": f.get("value"), "signature": "",
        }
    if tx_type == "SENSOR_VOUCH":
        return "/api/sensor/vouch", {"sensor_id": f["sensor_id"], "voucher": f["voucher"], "signature": ""}
    if tx_type == "GATEWAY_HEARTBEAT":
        return "/api/gateway/heartbeat", {"gateway_id": f["gateway_id"], "owner": f["owner"], "signature": ""}
    # NODE_ROLE_* bodies take `epoch` straight from the client (these handlers call
    # validate_and_apply directly rather than stamping s.chain.current_epoch() like
    # transfer/stake do) — but it's NOT part of the signed message, and
    # validate_and_apply enforces a stale-entry window (entry_epoch must be within
    # STALE_WINDOW of the chain's current epoch). There is no safe default: the
    # caller must fetch the current epoch (hone_node_health -> chain.current_epoch)
    # and pass it in `fields["epoch"]` right before submitting.
    if tx_type == "NODE_ROLE_OPT_IN":
        if "epoch" not in f:
            raise KeyError("NODE_ROLE_OPT_IN requires fields['epoch'] — fetch the current epoch via hone_node_health first")
        return "/api/node/role/opt-in", {
            "node": f["node"], "role": f["role"], "backer_share_bps": f["backer_share_bps"],
            "max_backers": f.get("max_backers", 20), "epoch": f["epoch"], "nonce": f["nonce"],
            "signed_by": f["node"], "signature": "",
        }
    if tx_type == "NODE_ROLE_OPT_OUT":
        if "epoch" not in f:
            raise KeyError("NODE_ROLE_OPT_OUT requires fields['epoch'] — fetch the current epoch via hone_node_health first")
        return "/api/node/role/opt-out", {
            "node": f["node"], "role": f["role"], "epoch": f["epoch"], "nonce": f["nonce"],
            "signed_by": f["node"], "signature": "",
        }
    if tx_type == "NODE_ROLE_STAKE":
        if "epoch" not in f:
            raise KeyError("NODE_ROLE_STAKE requires fields['epoch'] — fetch the current epoch via hone_node_health first")
        return "/api/node/role/stake", {
            "node": f["node"], "role": f["role"], "staker": f["staker"], "amount": f["amount"],
            "epoch": f["epoch"], "nonce": f["nonce"], "signed_by": f["staker"], "signature": "",
        }
    if tx_type == "NODE_ROLE_UNSTAKE":
        if "epoch" not in f:
            raise KeyError("NODE_ROLE_UNSTAKE requires fields['epoch'] — fetch the current epoch via hone_node_health first")
        return "/api/node/role/unstake", {
            "node": f["node"], "role": f["role"], "staker": f["staker"], "amount": f["amount"],
            "epoch": f["epoch"], "nonce": f["nonce"], "signed_by": f["staker"], "signature": "",
        }
    if tx_type == "ACCOUNT_UPDATE_KEY":
        return "/api/account/update-key", {
            "account": f["account"], "role": f["role"], "new_public_key": f["new_public_key"],
            "signed_by": f["account"], "signature": "",
        }
    if tx_type == "ACCOUNT_SET_PRIMARY":
        return "/api/account/set-primary", {"account": f["account"], "primary": f["primary"], "signed_by": f["account"], "signature": ""}
    if tx_type == "ACCOUNT_API_KEY_SET":
        return "/api/account/api-key", {
            "account": f["account"], "api_key": f["api_key"], "nonce": f["nonce"], "signed_by": f["account"], "signature": "",
        }
    if tx_type == "ACCOUNT_CREATE":
        return "/api/account/create", {
            "account": f["account"], "keys": f.get("keys") or {}, "chain_proofs": f.get("chain_proofs") or [],
            "funded_by": f.get("funded_by"), "signature": "",
        }
    raise UnsupportedTxType(f"no submit shape known for '{tx_type}'")


@mcp.tool()
async def hone_submit_signed_entry(endpoint: str, body: dict[str, Any]) -> dict[str, Any]:
    """Submit an already-signed transaction body to the node, exactly as returned by
    hone_prepare_signing_message (with `signature` filled in by your own signer).

    `endpoint` must be one of the submit_endpoint values hone_prepare_signing_message
    returns — this is a narrow pass-through, not a general HTTP proxy.
    """
    allowed = {
        "/api/transfer", "/api/stake", "/api/unstake", "/api/sensor/register",
        "/api/sensor/commit", "/api/sensor/vouch", "/api/gateway/heartbeat",
        "/api/node/role/opt-in", "/api/node/role/opt-out", "/api/node/role/stake",
        "/api/node/role/unstake", "/api/account/update-key", "/api/account/set-primary",
        "/api/account/api-key", "/api/account/create",
    }
    if endpoint not in allowed:
        return {"ok": False, "error": f"endpoint '{endpoint}' is not in the allowed submit list: {sorted(allowed)}"}
    try:
        return await _client().post(endpoint, body)
    except HoneApiError as e:
        return _err(e)


def main() -> None:
    mcp.run("stdio")


if __name__ == "__main__":
    main()
