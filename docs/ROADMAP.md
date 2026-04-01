# BTCPC Roadmap

## Phase 0 — Genesis (current)

- [x] Sovereign chain with 42M supply, 10 decimal precision
- [x] BIP-39 mnemonic wallets with multi-chain derivation (7 chains)
- [x] Proof of Useful Work mining (GPU + CPU)
- [x] P2P inference routing (async submit/poll)
- [x] Dynamic pricing (tokens × verified param count)
- [x] Verified reward splitting by actual parameter count
- [x] RAG — context documents with inference requests
- [x] MCP — user-specified tool servers, saved to profile
- [x] Telegram bots (thin HTTP clients via bot API)
- [x] Posting key signature verification for identity linking
- [x] Project registration, transfer, billing
- [x] Cross-chain wallet derivation on registration
- [x] Whitepaper v0.4 with consensus, RAG, MCP, MPC design
- [x] Genesis dreams (soulbound NFTs per block)
- [x] Shared epochs — multiple miners submit to same epoch, never double rewards
- [x] Finalization delay (60s) to wait for all miners before splitting

## Pending (user-requested, not yet scheduled)

- [ ] Wallet bot alerts only for users whose miner earned tokens (not broadcast to all)
- [ ] OpenClaw feature parity audit — identify missing features
- [ ] Inference cron jobs routed through real blockchain (not localhost)
- [ ] Auto-updater: stage updates, notify miner, apply on restart (not auto-apply)
- [ ] Systemd services for bots (auto-restart, no zombies)
- [ ] Cloudflare webhook mode for bots (eliminate polling entirely)
- [ ] Scrub git history to remove leaked .env / bot tokens

## Phase 1 — Multi-Miner

- [ ] 3-miner consensus per model (N=3 when 3+ miners serve a model)
- [ ] Work proof mempool — gossip proofs for block validation
- [ ] Variable block size (1 to thousands of proofs per epoch)
- [ ] Cross-chain signatures — accept ETH/Solana/TON keys as BTCPC auth
- [ ] Cascading bid system (escalate when initial bid rejected)
- [ ] Model auto-download on demand broadcast
- [ ] Streaming inference (/v1/chat/completions SSE)

## Phase 2 — Token Launch + Cross-Chain

- [ ] Cross-chain wallet watcher — monitor linked addresses on all 7 chains
- [ ] Proof of life: detect signed transactions from BTCPC-linked wallets
- [ ] Cross-chain reputation scoring (active wallets = higher trust)
- [ ] wBTCPC ERC-20 deployment on Base
- [ ] Bridge signer (shindevlin → multisig transition)
- [ ] Bridge transaction detection via chain watcher
- [ ] wBTCPC/USDC liquidity pool on Base DEX
- [ ] Solana wBTCPC SPL token + pool
- [ ] TON claim manager
- [ ] Purchase Hive account "shindevlin"
- [ ] npm publish @btcpc/sdk

## Phase 3 — Privacy

- [ ] MPC sharded inference (N miners, no single miner sees full data)
- [ ] Premium pricing tier (N× standard rate)
- [ ] Proof of Silicon — hardware attestation for miner verification
- [ ] Encrypted inference with client-side decryption

## Phase 4 — Maturity

- [ ] Explorer / dashboard UI
- [ ] Light client proofs for cross-chain verification
- [ ] Multisig bridge (replace single signer)
- [ ] Governance — token-weighted voting on protocol parameters
- [ ] E2E integration tests
- [ ] Third-party miner onboarding documentation
