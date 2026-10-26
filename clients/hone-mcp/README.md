# hone-mcp

An MCP (Model Context Protocol) server that wraps a running `hone-node`'s
local HTTP API so an agentic client — Claude Desktop, or any other MCP
client — can read chain state and drive node install/onboarding
conversationally, instead of through manual `curl`/systemd/CLI steps.

Built against `shindevlin/hone` as of 2026-10-25. Every endpoint path, body
field, and signing rule below was read directly from
`rust/hone-node/src/api.rs` and `rust/hone-node/src/tx.rs` in that repo —
nothing here is guessed. See `concerns.md` for chain/protocol-level issues
found along the way that are out of scope for this server to fix.

## Install

```bash
pip install -e .
# or, for a quick local run without installing:
pip install "mcp>=2.0.0" httpx
```

## Configure

Environment variables (all optional unless noted):

| Variable | Default | Used by |
|---|---|---|
| `HONE_NODE_URL` | `http://127.0.0.1:4242` | every tool (hone-node's documented default `HONE_API_PORT`) |
| `HONE_CHAIN_ID` | *(none)* | only `hone_prepare_signing_message`; must be `"hone"` (mainnet) or `"hone-testnet"` (testnet) — can also be passed per-call. The node has no endpoint that reports its own `chain_id`, so this server cannot discover it automatically (see `concerns.md`). |

Point `HONE_NODE_URL` at whichever node you're onboarding — typically the
one running on the same machine as this MCP server.

## Run

```bash
python -m hone_mcp.server
```

This speaks MCP over stdio. To use it from Claude Desktop, add it to
`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "hone": {
      "command": "python",
      "args": ["-m", "hone_mcp.server"],
      "env": { "HONE_NODE_URL": "http://127.0.0.1:4242" }
    }
  }
}
```

## What it does — security model

**This server never accepts, stores, derives, or transmits a private key,
BIP-39 mnemonic, or any signature-producing secret as a tool argument or
return value.** That shaped every design choice below, because the
alternative (an MCP tool that takes a raw key as a parameter) means a
private key passes through an LLM's context window and tool-call logs —
unacceptable for a chain wallet.

Four tiers of tools, in order of how much trust they require:

1. **Reads** — unrestricted. Chain state is public anyway (balances,
   accounts, stake, sensors, clock status, network snapshot).

2. **Bootstrap writes** — no signature needed, because the node's own
   verification code treats a role with no key registered yet as
   unauthenticated-by-design (`check_signature` returns `Ok` when the role
   being set has no key registered yet — the "grace period"). Only
   `hone_register_key` is in this tier. **`AccountCreate` is NOT** —
   live-tested against a real node on 2026-10-25, it unconditionally
   requires a signature from the owner key being registered
   (`check_account_create_signature` in `api.rs`/`tx.rs`; this is a
   deliberately stricter, separate check from the generic grace-period one,
   to prove the caller holds the owner key's private half before trusting
   it on-chain). `hone_create_account` is still zero-custody — it's just in
   tier 4 below, not here — and the fastest real path from "nothing
   installed" to "account exists with a posting key" is: generate an owner
   keypair yourself → `hone_prepare_signing_message` → sign it yourself →
   `hone_create_account` → `hone_register_key` for the posting key.

3. **Node self-sign writes** — no *external* signature needed, because the
   node signs with its own already-configured key
   (`HONE_POSTING_KEY`/`HONE_ACCOUNT` on the node process) and the key never
   leaves that process. Only two such endpoints exist on the node today:
   `/api/clock/self-register` and `/api/node/testnet/toggle`, exposed here
   as `hone_clock_self_register` and `hone_testnet_toggle`. These are the
   two highest-value onboarding actions this server unlocks: "make this
   machine participate in clock consensus" and "opt in to testnet operator
   rewards," both as single conversational calls, both with zero custody
   risk to this server.

4. **Signed writes** (new account creation, transfer, stake, node-role
   backing, sensor registration/commit, key rotation, etc.) — this
   server builds the message to sign and the body to submit
   (`hone_prepare_signing_message`), but does not sign it. Signing happens
   outside this process, with whatever ed25519 signer the operator already
   trusts (their existing `hone-transfer`/`hone-role-stake`/
   `hone-sensor-enroll` CLI helpers, a hardware key, etc.). The signature is
   then handed back in via `hone_submit_signed_entry`. This server is never
   in the signing loop for these.

### Why tier 4 doesn't cover every transaction type

The node's `canonical_signing_message()` function (in `tx.rs`) has two
different signing-message shapes:

- **Format A** (most entries): a hand-picked subset of client-known fields,
  serialized via `serde_json::json!({...})`. A naive read of the
  workspace's `Cargo.toml` files suggests this would serialize
  alphabetically (no crate here requests serde_json's `preserve_order`
  feature directly) — but a live test against the real compiled binary on
  2026-10-25 proved that's wrong: the `nostr` crate (a transitive
  dependency) requests `preserve_order` in its own `Cargo.toml`, and Cargo
  unifies features workspace-wide, so it's silently on for every
  `serde_json` use including this one. The real order is **insertion
  order** — exactly the field order written in each `json!({...})` call.
  `signing.py` reproduces this by building each message's fields as a
  Python dict literal in that same order (no `sort_keys`). See
  `signing.py`'s module docstring and `concerns.md` finding #0 — this is
  an accident of an unrelated dependency, not a documented contract, and
  could silently flip on a future `Cargo.lock` change with nothing in the
  hone repo to catch it.
- **Format B** (the catch-all `other =>` arm — everything not given an
  explicit Format-A arm, notably `ClockNodeRegister`): signs
  `{"chain_id": ..., "entry": <typed serde serialization of the full entry,
  struct-declaration field order, "type" tag first, NOT alphabetical>}`.
  Reproducing this exactly in Python means matching Rust's struct
  serialization field order byte-for-byte, with no margin for error (a
  wrong order produces a signature that fails to verify, silently, with no
  diagnostic pointing at "field order"). This server does **not**
  implement Format B — and doesn't need to, because the two Format-B
  entries relevant to onboarding (`ClockNodeRegister`,
  `TestnetOperatorRegister`) both already have the node self-sign endpoints
  in tier 3 above. `hone_prepare_signing_message` rejects a Format-B
  `tx_type` with a message pointing at the right tool instead of producing
  a signature that looks valid but silently fails.

`signing.py`'s module docstring has the full citation trail (source
file/function names) for anyone verifying this against a future version of
the node.

### Two LedgerEntry variants with no HTTP route at all

`SensorKeyRegister` and `DeviceKeyRegister` are real ledger entry types
with their own Format-A signing arms, but as of this read of `api.rs`
neither has a `.route(...)` registered, and there's no generic raw-entry
submission endpoint either. `hone_prepare_signing_message` refuses both
with an explicit "not submittable yet" error rather than returning a
plausible-looking message for an endpoint that would 404. See
`concerns.md`.

## Tool list

Reads: `hone_node_health`, `hone_get_balance`, `hone_get_all_balances`,
`hone_get_account`, `hone_get_account_history`, `hone_get_stake`,
`hone_staking_requirements`, `hone_node_role_positions`, `hone_clock_status`,
`hone_clock_uptime`, `hone_list_sensors`, `hone_get_sensor`,
`hone_explorer_status`, `hone_faucet_status`, `hone_testnet_status`.

Bootstrap write (no signature at all): `hone_register_key`.

Node self-sign writes (no external signature): `hone_clock_self_register`,
`hone_testnet_toggle`.

Signed writes: `hone_create_account` (needs one signature from the new owner
key — see "Security model" above), and the generic
`hone_prepare_signing_message` / `hone_submit_signed_entry` pair for
everything else (transfer, stake, node-role backing, sensor
registration/commit, key rotation on an already-keyed account, etc.).

## Scope

This is a v1 slice focused on what makes install/onboarding agentic:
status checks, account bootstrap, and the two zero-custody self-sign
flows. It is not a full client SDK for every one of the ~150 `LedgerEntry`
variants on the chain (inference jobs, LinkGit, governance, tracker,
freeport, etc.) — those can be added the same way (read the real handler
in `api.rs`, read the real signing arm in `tx.rs`, don't guess) as they
become relevant to an onboarding or operating flow.
