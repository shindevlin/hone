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
- [ ] **Memo key as universal inbox**: memo key pubkey stored on-chain at account registration; used for end-to-end encryption of reputation memos, digital goods delivery, and private order data — one key, one inbox
- [ ] **Memo key reputation system**: after ORDER_DELIVERED both parties may write a REPUTATION_MEMO entry — buyer-to-seller and seller-to-buyer signed memos, memo text encrypted with the subject's memo key and stored as a BTCPC-FS blob, `vote` field (+1/-1/0) is public
- [ ] **Buyer staking flow**: buyer stakes BTCPC via STAKE_LOCK entry; staked balance acts as a pre-authorized escrow pool — orders debit from the pool without requiring an active key per transaction; 4,800-epoch (40h) cooldown on unlock
- [ ] **Multi-sig escrow**: 2-of-3 scheme (buyer + seller + protocol) for dispute resolution; winning party receives escrowed amount, losing party's stake reduced proportionally; nothing burned
- [ ] **Owner key + KEY_ROTATE / KEY_REVOKE entry types**: root-of-trust key that can rotate any other key; cold-storage only; on-chain key rotation is permanent and immediate across all validators

## Phase H2 — Service Keys & Digital Delivery

- [ ] **Service key delegation**: `SERVICE_KEY_DELEGATE` entry (signed by posting key) authorizes a service node's Ed25519 pubkey to sign `HEARTBEAT`, `SERVICE_LOG`, `SERVICE_RESULT`; optional `service_image_cid` binds the key to a specific WASM binary; `SERVICE_KEY_REVOKE` invalidates immediately
- [ ] **Fulfill key**: `FULFILL_KEY_REGISTER` entry (signed by posting key) registers a fulfill pubkey; fulfill privkey is stored AES-encrypted in BTCPC-FS using ECDH(fulfill_privkey, service_pubkey) — only the authorized service node can decrypt it; scoped to `ORDER_FULFILL` on `auto_deliver` products only
- [ ] **Digital goods delivery encryption — on-chain buyer**: at ORDER_FULFILL, service ECDH-encrypts delivery content to buyer's memo pubkey; buyer decrypts with memo privkey; end-to-end, no plaintext on any node, permanent
- [ ] **Digital goods delivery encryption — guest buyer with password**: browser derives Ed25519 keypair from HKDF(password, order_id); pubkey included in ORDER_PLACE; server never sees password; same ECDH fulfill path as on-chain buyer; buyer re-derives key from password + order ID at any future time
- [ ] **Digital goods delivery encryption — guest buyer no account**: fulfill service issues a signed time-limited download token (signed by fulfill key); valid for 4,800 epochs; authenticated but not end-to-end encrypted; honest about the guarantee boundary
- [ ] **Content encryption at rest**: `auto_deliver` product content stored encrypted in BTCPC-FS at listing time; fulfill key decrypts raw content then re-encrypts addressed to buyer's specific delivery pubkey; plaintext never exists on network after initial vendor upload

## Phase I — Discovery & Search

- [ ] Full-text product search across all sellers, indexed from the P2P ledger
- [ ] Category browsing with cross-seller result aggregation
- [ ] Featured stores and trending products (ranked by order volume and reputation)
- [ ] Store analytics dashboard with real ledger data: actual revenue, order counts, repeat buyer rate
- [ ] Seller verification badge — stake-weighted reputation threshold required to unlock badge
- [ ] **Saved searches + price-drop alerts**: buyer saves a search query or wishlist item; on-chain price-change events trigger a notification (Telegram or in-app); no ML, pure query matching
- [ ] **Price guide from completed orders**: for hardware categories, aggregate median/low/high of completed orders by `item_model` tag over the last 90 epochs and display as a price guide on listing pages (Reverb pattern)
- [ ] **Follow sellers + buyer feed**: buyers follow seller accounts; new listings from followed sellers appear in a chronological feed endpoint (`GET /api/commerce/feed`) — social discovery without a recommendation engine
- [ ] **Public wishlist**: buyer marks listings as wished; wishlist is public and queryable; seller can see who has wishlisted their products and target discount codes at them

## Phase I2 — Listing & Checkout UX

- [ ] **Pay-what-you-want with floor**: `min_price` + `suggested_price` fields on listings; buyer sees the suggested amount pre-filled but can raise it; floor is enforced on-chain at ORDER_PLACE (Gumroad/Bandcamp pattern) — particularly useful for AI model weights and guides
- [ ] **Offer / counter-offer**: buyer submits `OFFER_PLACE` entry (signed, amount + product_id, expires at `created_epoch + 2880`); seller accepts (`OFFER_ACCEPT` → becomes ORDER_PLACE) or counters (`OFFER_COUNTER`); all offers are on-chain signed messages (eBay/Reverb/Depop pattern)
- [ ] **Early Access listings**: `expected_delivery_epoch` field on listings; listing shows "Expected: Epoch ~N"; payment held in escrow; auto-refunded if seller misses delivery epoch by 4,800 epochs (Steam Early Access pattern)
- [ ] **Tiered product listings**: single listing page with multiple price tiers (e.g., Basic / Pro / Source) each unlocking a different delivery CID; one ORDER_PLACE references the selected tier (Gumroad pattern — useful for quantized vs full-precision model weights)
- [ ] **Bundle orders**: buyer adds multiple products from one seller, proposes a single total price; `BUNDLE_ORDER` entry links multiple product_ids with one payment; seller accepts or counters (Depop/Newegg pattern)
- [ ] **Listing bump for visibility**: seller pays X dreams to `btcpc_recycle`; listing gets `bumped_until_epoch` field; marketplace sorts bumped listings above non-bumped in the same category (Reverb pattern)
- [ ] **Condition taxonomy for hardware**: enforced condition tiers (New / Like New / Very Good / Good / Fair) with mandatory condition notes and minimum one photo per non-new listing; validated at PRODUCT_CREATE (Newegg/Reverb pattern)
- [ ] **Technical spec table**: structured `specs: {}` JSON field on listings for hardware; buyer-facing comparison view for up to 4 listings side-by-side; no ML required (Newegg pattern)
- [ ] **Deterministic license key for software**: on ORDER_FULFILL, generate `sha256(order_id + seller_posting_pubkey)` as a license key; included in ORDER_FULFILL entry; buyer can re-derive it from their order ID at any time — no key server (Gumroad pattern)
- [ ] **Multi-image gallery**: listings support multiple image CIDs stored in BTCPC-FS; displayed as a swipeable gallery; minimum one image required for hardware listings (Depop pattern)
- [ ] **Shop share page with Open Graph tags**: static `/shop/:username` page with og:title, og:image, og:description populated from seller's on-chain store data — one-tap share to any social platform with a preview card
- [ ] **Tip / above-floor payment**: optional tip input at checkout for digital goods; buyer can pay more than the listing price; `tip_amount` recorded separately in ORDER_PLACE; goes to seller in full (Bandcamp pattern)

## Phase I3 — Trust & Reputation

- [ ] **Structured review sub-scores**: reviews record `{ value: N, quality: N, accuracy: N, speed: N }` alongside the single aggregate star; aggregate and per-dimension scores shown on store and product pages (Newegg Egg Rating pattern)
- [ ] **Seller response to reviews**: seller may post one public reply to any review; stored as an append-only entry linked to the original REPUTATION_VOTE; visible on the product page (eBay pattern)
- [ ] **Seller performance scorecard**: `late_fulfill_rate`, `dispute_rate`, `avg_fulfill_epochs` computed from the seller's on-chain order history and displayed on the store page — auditable metrics, not just stars (Amazon seller scorecard pattern)
- [ ] **Purchase protection escrow pool**: 0.25% of every order total routes to a `btcpc_commerce_protection` account; funds used to compensate verified claims of non-delivery or significant-not-as-described; governed by staked arbiters (Etsy purchase protection pattern)
- [ ] **Seller handling time commitment**: seller sets `handling_epochs` at the store level; per-listing overrides allowed; buyer-facing deadline ("Ships by epoch N") is computed and displayed; late fulfillment increments `late_fulfill_rate` on-chain (eBay handling time pattern)

## Phase I4 — Live Commerce

- [ ] **Epoch-clock auctions**: `AUCTION_OPEN` listing type with `auction_end_epoch`; all nodes see the same epoch clock so the countdown is trustless; `BID_PLACE` entries replace fixed-price ORDER_PLACE; highest bid at `auction_end_epoch` wins; losing bids auto-refunded (Whatnot/eBay pattern)
- [ ] **Verifiable giveaway**: during a live sale, `GIVEAWAY_DRAW` entry uses the epoch block hash as randomness seed to select a winner from all registered participants — publicly auditable, no trust in the seller (Whatnot pattern)
- [ ] **Pre-authorized live checkout**: buyer pre-signs an escrow lock up to amount N with a TTL; during a live sale event they can claim items without a per-purchase confirmation step; authorization expires at `auth_end_epoch` (Whatnot one-tap pattern)

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
