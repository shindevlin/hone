# API Reference

## Endpoints

### User
- `POST /api/register` — Register a new user
- `POST /api/login` — Authenticate and receive a session token
- `POST /api/link-telegram` — Link Telegram account
- `POST /api/enable-2fa` — Enable 2FA for account

### Wallet
- `GET /api/wallet/balance` — Get token balances
- `POST /api/wallet/transfer` — Transfer tokens (2FA required for Telegram-linked users)

### Staking
- `GET /api/staking/pools` — List available staking pools
- `POST /api/staking/stake` — Stake tokens
- `POST /api/staking/unstake` — Unstake tokens
- `POST /api/staking/claim` — Claim staking rewards

### Blockchain Integration
- `POST /api/blockchain/link` — Link Hive or TON account
- `POST /api/blockchain/send` — Send tokens on Hive or TON

## Authentication
- Most endpoints require a valid session token (JWT or similar) in the `Authorization` header.
- 2FA is enforced for token-moving actions for Telegram-linked users. Provide the 2FA code in the request body or as required.

## Error Handling
- Standard error responses include an HTTP status code and a JSON body:
  ```json
  {
    "error": "Invalid2FA",
    "message": "2FA code is required for this action."
  }
  ```
- Common errors:
  - `Invalid2FA`: 2FA code missing or incorrect
  - `Unauthorized`: Invalid or missing session token
  - `InsufficientBalance`: Not enough tokens to complete action
  - `PoolNotFound`: Staking pool does not exist

For more details and request/response examples, see the [Getting Started Guide](getting-started.md) and [Module Documentation](modules.md). 