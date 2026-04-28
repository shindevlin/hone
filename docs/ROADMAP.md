# BTCPC Roadmap

## Phase 0 — Genesis (complete)

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

## Phase G — Commerce (complete)

- [x] `btcpc-market` Rust/Axum sidecar (port 7042)
- [x] Stores: open/update/close with on-chain signed ledger entries
- [x] Products: create/update/delist; inventory tracking; flash sale pricing
- [x] Auto-deliver digital goods via BTCPC-FS CID at order placement
- [x] Orders: ORDER_PLACE / ORDER_FULFILL / ORDER_DELIVERED / ORDER_CANCEL / ORDER_DISPUTE
- [x] Escrow: social commitment on-chain; active key debit is Phase H
- [x] 40-hour auto-cancel sweep for unfulfilled orders (4,800 epochs)
- [x] Shipping account integration: UPS, FedEx, USPS, DHL, PirateShip
- [x] Tor hidden-service setup registered on-chain; buyer auto-routes
- [x] Q&A per product listing; public read, auth-gated ask/answer
- [x] Reputation votes; verified-buyer gate (must have a delivered order)
- [x] P2P catalog mirror: every node serves `GET /api/peer/commerce/*`
- [x] Posting key auth (JWT or X-Posting-Key header)
- [x] BTCPC-FS content-addressed blob storage (sha256 CIDs)

## Pending (user-requested, not yet scheduled)

- [ ] Wallet bot alerts only for users whose miner earned tokens (not broadcast to all)
- [ ] OpenClaw feature parity audit — identify missing features
- [ ] Inference cron jobs routed through real blockchain (not localhost)
- [ ] Auto-updater: stage updates, notify miner, apply on restart (not auto-apply)
- [ ] Systemd services for bots (auto-restart, no zombies)
- [ ] Cloudflare webhook mode for bots (eliminate polling entirely)
- [ ] Scrub git history to remove leaked .env / bot tokens

## Phase H — Auth & Wallet Integration (next)

- [ ] **Active key escrow debit**: ORDER_PLACE signs an ESCROW_LOCK entry debiting the buyer's wallet on-chain using their active key — turns social commitment into a real fund hold
- [ ] **Active key escrow release**: ORDER_DELIVERED triggers an ESCROW_RELEASE entry signed by the buyer's active key, sending held funds to the seller minus protocol fee
- [ ] **Memo key reputation system**: after ORDER_DELIVERED both parties may write a REPUTATION_MEMO entry — buyer-to-seller and seller-to-buyer signed memos, memo text encrypted with the subject's memo key and stored as a BTCPC-FS blob, `vote` field (+1/-1/0) is public
- [ ] **Buyer staking flow**: buyer stakes BTCPC via STAKE_LOCK entry; staked balance acts as a pre-authorized escrow pool — orders debit from the pool without requiring an active key per transaction; 4,800-epoch (40h) cooldown on unlock
- [ ] **Multi-sig escrow**: 2-of-3 scheme (buyer + seller + protocol) for dispute resolution; winning party receives escrowed amount, losing party's stake reduced proportionally; nothing burned

## Phase I — Discovery & Search

- [ ] Full-text product search across all sellers, indexed from the P2P ledger
- [ ] Category browsing with cross-seller result aggregation
- [ ] Featured stores and trending products (ranked by order volume and reputation)
- [ ] Store analytics dashboard with real ledger data: actual revenue, order counts, repeat buyer rate
- [ ] Seller verification badge — stake-weighted reputation threshold required to unlock badge

## Phase J — Payments & Tokens

- [ ] Wrapped BTCPC (wBTCPC) bridge to Ethereum ERC-20 (Base deployment)
- [ ] Multi-token checkout: accept any token with liquidity, settled to BTCPC via bonding curve at order time
- [ ] Discount codes and bundle pricing as on-chain entry types
- [ ] Subscription products: recurring ORDER_PLACE at a fixed epoch interval, buyer-authorized
- [ ] Affiliate/referral system: referrer address gets a configurable % of order fee logged on-chain as a REFERRAL_CREDIT entry

## Phase K — Infrastructure & Scale

- [ ] Docker Compose for single-validator deployment: btcpc-node + btcpc-market + nginx in one `docker compose up`
- [ ] Kubernetes for public gateway tier: btcpc-market replicas behind ingress, shared pending-entries via distributed log (Kafka or NATS)
- [ ] BTCPC-FS CDN: content-addressed blob delivery from multiple storage nodes; buyers pull from nearest host
- [ ] Per-seller Telegram notifications: `telegram_chat_id` field on STORE_UPDATE so each vendor registers their own bot chat — today's global `BTCPC_TELEGRAM_CHAT_ID` env var is an admin fallback only; per-seller takes precedence when set
- [ ] Mobile order tracking in the Android app: buyer sees live order status pulled from any BTCPC node

## Phase L — Governance & Compliance

- [ ] On-chain dispute resolution: DISPUTE_ARBITRATE entry type; arbiters are staked validators elected by governance vote; losing party's stake is reduced
- [ ] BTCPC Verified Seller program: stake threshold + minimum review count + minimum on-chain order history required for program badge
- [ ] Privacy mode: all order data (shipping address, item detail) encrypted with buyer/seller memo keys; only the two parties can read; ledger records only hashes

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
- [ ] Private authorization stack — BTCPC spends approved by Bitcoin, Lightning, zkVM, or other supported chains
  - Implementation plan: [docs/PRIVATE_AUTH_IMPLEMENTATION_PLAN.md](PRIVATE_AUTH_IMPLEMENTATION_PLAN.md)
- [ ] BTCPC-native ZK verifier backend for private authorization receipts

## Phase 4 — Maturity

- [ ] Explorer / dashboard UI
- [ ] Light client proofs for cross-chain verification
- [ ] Multisig bridge (replace single signer)
- [ ] Governance — token-weighted voting on protocol parameters
- [ ] E2E integration tests
- [ ] Third-party miner onboarding documentation
