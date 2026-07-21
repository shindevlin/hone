# HONE Network Endpoints

All agents, bots, and nodes MUST use these endpoints to query chain state.
Never use `localhost` or `127.0.0.1` — local node state may be behind or on a different chain.
All listed nodes are equal peers. There is no primary node — this is a blockchain.

## Mainnet (`hone-1`)

### Peer Nodes

| Node | URL |
|------|-----|
| Grouchly (shindevlin) | `http://192.168.68.72:4242` |
| Nebra (natoshisakamoto) | `http://192.168.68.75:4242` |

### Endpoints (any peer)

| Purpose | Endpoint |
|---------|----------|
| Balance | `GET /api/balance/{account}` |
| Account info | `GET /api/account/{account}` |
| Node info / epoch | `GET /api/node/info` |
| Submit transfer | `POST /api/transfer` |
| Post inference job | `POST /api/inference/post` |

## Testnet (`hone-satoshi`)

### Peer Nodes

| Node | URL |
|------|-----|
| Grouchly (shindevlin) | `http://192.168.68.72:4343` |
| Nebra (natoshisakamoto) | `http://192.168.68.75:4343` |

### Endpoints (any peer)

| Purpose | Endpoint |
|---------|----------|
| Faucet (testnet only) | `POST /api/faucet/claim` — body: `{"account":"yourname"}` |

## Chain IDs

| Chain | ID | Genesis timestamp |
|-------|----|-------------------|
| Mainnet | `hone-1` | `1777672500000` |
| Testnet | `hone-satoshi` | `1777672500000` |

## Usage examples

```bash
# Check any account balance on mainnet
curl http://192.168.68.72:4242/api/balance/shindevlin

# Check natoshisakamoto on mainnet
curl http://192.168.68.72:4242/api/balance/natoshisakamoto

# Testnet faucet
curl -X POST http://192.168.68.72:4343/api/faucet/claim \
  -H "Content-Type: application/json" \
  -d '{"account":"yourname"}'

# Current epoch / node state
curl http://192.168.68.72:4242/api/node/info
```

## Notes

- Signing messages use BTreeMap key order (alphabetical). Always `sort_keys=True` in Python.
- 1 HONE = 10,000,000,000 dreams
- Transfer canonical message fields: `amount, chain_id, from, nonce, to, token, type`
- Nonces are per-account and increment with every signed transaction.
