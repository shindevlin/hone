# Getting Started Guide

## Prerequisites
- Node.js (v16+ recommended)
- npm or yarn
- MongoDB or your chosen database (see .env.example)
- Telegram account (for 2FA/Telegram features)
- Hive or TON account (for blockchain integration)

## Installation
1. Clone the repository:
   ```sh
   git clone https://github.com/your-org/urs-nerdcore-node.git
   cd urs-nerdcore-node
   ```
2. Install dependencies:
   ```sh
   npm install
   # or
   yarn install
   ```
3. Configure environment:
   - Copy `.env.example` to `.env` and fill in required values (API keys, DB connection, etc.)
4. Start the node:
   ```sh
   npm start
   # or
   yarn start
   ```

## First Use
- Register a new user via the API or UI.
- Link your Telegram account (if using Telegram features).
- Enable 2FA in your account settings (required for token-moving actions).
- Link your Hive or TON account for blockchain features.

## Example Flows
### Sending Tokens
- Use the wallet API to transfer tokens to another user or address.
- 2FA will be required for Telegram-linked users.

### Staking
- Join a staking pool via the staking API.
- Stake tokens and monitor rewards.
- Unstake and claim rewards as needed.

### Viewing Balances
- Use the wallet API to view your token balances (no 2FA required for read-only actions).

For more details, see the [Module Documentation](modules.md) and [API Reference](api.md). 