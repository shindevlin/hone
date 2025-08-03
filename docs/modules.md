# Module Documentation

## UnifiedAccountModule
Handles user account management, including:
- Telegram account linking
- 2FA enforcement for token-moving actions (transfers, staking, spending)
- User authentication and session management

**Usage:**
- Injected into wallet, staking, and other modules to enforce security policies.
- Example: `enforceTelegram2FAForUser(user)`

## Wallet Module
Manages token balances and transfers:
- Multi-chain support (Hive, TON)
- Secure token transfers (2FA required for Telegram-linked users)
- Read-only actions (view balances, NFTs) do not require 2FA

**Usage:**
- Transfer tokens: `wallet.transfer(from, to, amount)`
- View balances: `wallet.getBalance(user)`

## Staking Module
Provides staking pool functionality:
- Create and manage staking pools
- Stake/unstake tokens
- Claim rewards
- Customizable APY and reward logic

**Usage:**
- Create pool: `staking.createPool(params)`
- Stake: `staking.stake(user, poolId, amount)`
- Claim rewards: `staking.claim(user, poolId)`

## Hive/TON Integration
Blockchain connectivity and token operations:
- Hive Engine: Token creation, transfers, staking
- TON: Token transfers, wallet integration
- Easy configuration for new chains

**Usage:**
- Link blockchain account: `blockchain.linkAccount(user, chain)`
- Send tokens: `blockchain.sendToken(user, to, amount, chain)`

## Example Usage
```js
// Transfer tokens with 2FA enforcement
if (unifiedAccountModule.enforceTelegram2FAForUser(user)) {
  wallet.transfer(user, recipient, amount);
}

// Stake tokens
staking.stake(user, poolId, amount);

// View balances
wallet.getBalance(user);
``` 