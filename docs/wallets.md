# HONE Wallet & Account System

## Account Model

Every HONE account has:
- **Username** — unique, permanent, human-readable (e.g. `shindevlin`, `josh`)
- **12-word BIP-39 mnemonic** — the master secret that derives everything
- **4 key pairs** (owner, active, posting, memo) — derived deterministically from the mnemonic
- **Cross-chain addresses** — deterministically derived for HONE, EVM, Solana, Bitcoin, TON, Hive

## Key Hierarchy

```
12-Word BIP-39 Mnemonic
  |
  +-- m/44'/hone'/0'/0/0  --> Owner Key    (change keys, recovery)
  +-- m/44'/hone'/0'/1/0  --> Active Key   (transfers, staking)
  +-- m/44'/hone'/0'/2/0  --> Posting Key  (social ops, bot linking)
  +-- m/44'/hone'/0'/3/0  --> Memo Key     (encrypted messages)
```

All keys are secp256k1. Given the same mnemonic, the same keys and addresses are always derived.

## Creating an Account

### Via Code
```javascript
const { createAccount } = require('./src/wallet/accountManager');
const result = await createAccount('username', null, 'password');
// result.mnemonic — SAVE THIS, it's shown once
// result.address — HONE address
// result.publicKeys — { owner, active, posting, memo }
```

### Via CLI
```bash
node bin/hone-cli wallet create <username>
```

### Recovering Keys from Mnemonic
```javascript
const keyManager = require('./src/wallet/keyManager');
const keys = await keyManager.mnemonicToKeys(mnemonic);
// keys.owner.privateKey, keys.owner.publicKey
// keys.active.privateKey, keys.active.publicKey
// keys.posting.privateKey, keys.posting.publicKey
// keys.memo.privateKey, keys.memo.publicKey

const wallets = await keyManager.deriveChainWallets(mnemonic);
// wallets.evm.address, wallets.solana.address, etc.
```

## Key Permissions

| Key | Can Do | Cannot Do |
|-----|--------|-----------|
| **Owner** | Change other keys, full recovery | — |
| **Active** | Transfer tokens, stake, delegate | Change keys |
| **Posting** | Link Telegram, social ops | Move money |
| **Memo** | Decrypt private messages | Anything else |

## Sending Tokens

### Via CLI
```bash
HONE_MINER=<username> HONE_ACTIVE_KEY=<active_private_key> \
  node bin/hone-cli wallet send <recipient> <amount>
```

### Via API
```bash
curl -X POST http://localhost:3000/api/wallet/transfer \
  -H "Authorization: Bearer <jwt>" \
  -H "Content-Type: application/json" \
  -d '{"toAddress": "<recipient_address>", "amount": 10}'
```

## Transaction Flow

1. Sender submits transfer (API or CLI)
2. **Mempool validation** — check balance, reject double-spends via nonce
3. **Transaction hash** — deterministic SHA-256 of canonical tx data
4. **P2P broadcast** — TRANSACTION message gossips to all peers immediately
5. **Wallet caches update** — sender and receiver balances reflect immediately
6. **Ledger write** — permanent LedgerEntry created (source of truth)
7. **Block inclusion** — next EPOCH_FINALIZED includes the entry in a block file
8. **SMT update** — state root in block header reflects the new balances

## Storage

- **Mnemonics** — `.env` file only, never in git
- **Private keys** — derivable from mnemonic, stored in `.env` for convenience
- **Public keys** — on the permanent ledger (LedgerEntry with type ACCOUNT_CREATE)
- **Balances** — computed from the ledger: `sum(incoming) - sum(outgoing)`
- **Wallet.balance** — MongoDB cache for fast reads, NOT the source of truth

## Environment Variables

```bash
# Genesis miner (shindevlin)
HONE_MNEMONIC=<12 words>          # Used by createGenesisBlock
HONE_POSTING_KEY=<hex>             # For claim signing

# Any miner
HONE_MINER=<username>              # Which account to mine as
HONE_ACTIVE_KEY=<hex>              # For signing transactions
```
