# HONE Changelog

## v2.0.37 — Settlement-based rewards
- Jobs settle when all required verifications are in (3 in consensus, 1 in genesis)
- Miners get paid in the epoch the job SETTLES, not when they did the work
- Empty epochs don't consume emission slots — reward defers to next epoch with work
- `epochs_deferred` tracks how far behind the reward schedule is
- InferenceJob schema: verifications[], settlement_epoch, full timing data

## v2.0.36 — MINER_IDLE broadcast
- Miners announce idle via P2P when no work for an epoch
- Finalization counts idle + proofs before triggering (no wasted waiting)

## v2.0.35 — Finalize after last miner
- Finalization polls for all active miners' proofs instead of dumb 60s timer
- Last miner to submit triggers finalization

## v2.0.34 — P2P proof gossip
- MINING_PROOF message type — miners broadcast proofs via P2P
- Receiving nodes store remote proofs locally for finalization
- Eliminates need for shared database between miners

## v2.0.33 — Epoch sync fix (P2P height)
- Epoch sync uses MAX(time, DB, P2P chain height)
- Fixes miners stuck at wrong epoch when P2P chain is ahead of local DB

## v2.0.32 — Epoch sync fix (DB height)
- Epoch sync checks highest epoch in DB, not just time-based calculation

## v2.0.31 — Auto model manager
- HONE_MAX_MODEL_STORAGE_GB — disk budget for models
- Auto-pulls highest-demand models that fit within budget
- Checks model size from registry before downloading
- GLM-OCR added to Grouchly for document/image processing

## v2.0.30 — P2P job failure fix
- Only the miner who claimed a job can fail it
- Other miners' errors don't overwrite active jobs

## v2.0.29 — Local inference + auto model picker
- `local: true` — direct Ollama, no P2P, no rewards, no billing
- `model: "auto"` — picks model by prompt length (<200=4b, <2000=9b, else 27b)

## v2.0.28 — Telegram-notified updates
- Auto-updater stages code, notifies miner via Telegram
- `/approve-update` command to restart — no auto-restarts
- Cross-platform (Windows + Linux)

## v2.0.27 — Model hash verification
- SHA-256 blob hash verified against Ollama registry
- Tampered models rejected — miner refuses to mine
- Rejected proofs get 0 reward, share goes to replacement miner
- model_hash stored on MiningProof

## v2.0.26 — Handler fixes
- Exact model match before claiming (qwen3:4b != qwen3.5:27b)
- Handler uses HONE_MINER env var (was hardcoded to genesis miner)
- Auto-updater Windows compatibility

## v2.0.25 — Shared epochs + reward splitting
- Multiple miners submit to same epoch, NEVER double rewards
- Finalization delay waits for all miners before splitting
- Reward split by work_value (tokens x verified_param_count)
- MCP tool execution (inline servers, no registration)
- Pricing normalized for raw param counts

## v2.0.24 — RAG + MCP + MPC
- RAG: context field on inference submit — documents prepended as system message
- MCP: user-specified tool servers, saved to profile, permissionless
- MPC: designed in whitepaper (sharded privacy, Nx cost)
- Removed all "Beastly" name references

## v2.0.23 — Verified param-count rewards
- getModelWeight queries Ollama /api/show for general.parameter_count
- work_value = tokens x actual_param_count (not model name parsing)
- Prevents gaming by naming fake models with large param counts

## v2.0.22 — Configurable miner identity
- HONE_MINER env var — mine as any registered account
- Grouchly mines as natoshisakamoto, shindevlin's node as shindevlin

## v2.0.21 — Bot API routes
- 23 /api/bot/* endpoints — bots are thin HTTP clients
- No direct MongoDB access from bots
- x-bot-key authentication

## v2.0.20 — Posting key signature verification
- /link requires signing challenge with HONE posting key
- secp256k1 ecdsaVerify against postingPublicKey in DB
- CLI: sign-challenge command for local signing
- On-chain verification transaction recorded with signature

## v2.0.19 — Bot /verify command + docs
- /verify command completes Telegram linking in chat
- docs/bots.md with token management, zombie prevention
- CLAUDE.md references bots.md

## v2.0.18 — Project transfer
- POST /api/projects/transfer — sell your business, keep your identity
- API key rotates on transfer, old owner loses access

## v2.0.17 — Multi-chain wallet derivation
- Registration derives wallets for all 7 chains from single BIP-39 mnemonic
- EVM (keccak256), Solana (ed25519), Bitcoin (bech32), TON (ed25519)
- User imports mnemonic into MetaMask/Phantom/Electrum
- deps: ed25519-hd-key, js-sha3, bech32, bs58
