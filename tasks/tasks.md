# URSNode Task Tracker

This file tracks all tasks for the BTCPC project.

## Task Management System
- All tasks are tracked here for this repository
- Tasks are prioritized based on business value and urgency
- Status: todo | in_progress | blocked | completed
- Each task has a clear success criterion

## Task Status Legend
- [ ] Todo
- [x] Completed
- [>] In Progress
- [!] Blocked

---

## 🚨 CRITICAL (Must Complete Before Testing)

### C1: Project Infrastructure Setup
- [x] Create `package.json` with proper dependencies (Express, MongoDB, JWT, crypto, etc.)
- [x] Create `.env.example` with all required environment variables
- [x] Initialize Git repository with proper structure
- [x] Set up basic project layout (src/, tests/, config/)

### C2: Core Authentication & Users
- [ ] Implement `user.js` - User model and registration endpoint
  - Status: todo
  - Success criterion: Users can register and log in via API
- [ ] Implement `auth.js` - JWT authentication middleware
  - Status: todo
  - Success criterion: API routes are protected by JWT tokens
- [ ] Implement `telegram-link.js` - Telegram account linking
  - Status: todo
  - Success criterion: Users can link their Telegram accounts to their URSNode accounts
- [ ] Implement `2fa.js` - 2FA generation and verification for sensitive actions
  - Status: todo
  - Success criterion: 2FA can be enabled and verified for security-sensitive operations

### C3: Database Layer
- [ ] Create `models/User.js` - User schema with Telegram ID, 2FA status
  - Status: todo
  - Success criterion: User data is properly stored in MongoDB with all required fields
- [ ] Create `models/Wallet.js` - Multi-chain wallet balances
  - Status: todo
  - Success criterion: Wallet data is stored with support for multiple chains (Hive, TON)
- [ ] Create `models/StakingPool.js` - Staking pool configuration
  - Status: todo
  - Success criterion: Staking pool data can be stored and retrieved
- [ ] Set up MongoDB connection with proper error handling
  - Status: todo
  - Success criterion: Application can connect to MongoDB with retry logic and proper error reporting

---

## 🔥 HIGH (Core Functionality)

### H1: Wallet Module (Priority #1)
- [ ] Implement `wallet/balance.js` - Get token balances across chains
  - Status: todo
  - Success criterion: API endpoint returns accurate token balances from Hive and TON blockchains
- [ ] Implement `wallet/transfer.js` - Transfer tokens with 2FA check
  - Status: todo
  - Success criterion: Users can transfer tokens between addresses with 2FA verification
- [ ] Implement `wallet/nfts.js` - View NFT collections (read-only, no 2FA)
  - Status: todo
  - Success criterion: NFT collections can be retrieved and displayed
- [ ] Create Hive wallet integration adapter
  - Status: todo
  - Success criterion: Seamless integration with Hive blockchain for wallet operations
- [ ] Create TON wallet integration adapter
  - Status: todo
  - Success criterion: Seamless integration with TON blockchain for wallet operations

### H2: Blockchain Integration (Priority #2)
- [ ] Create `blockchain/hive.js` - Hive Engine API client
  - Status: todo
  - Success criterion: Reliable API client for interacting with Hive Engine smart contracts
- [ ] Create `blockchain/ton.js` - TON API client
  - Status: todo
  - Success criterion: Reliable API client for interacting with TON blockchain
- [ ] Implement account linking endpoints for both chains
  - Status: todo
  - Success criterion: Users can link their Hive and TON accounts to their URSNode profile
- [ ] Implement token transfer functions with proper error handling
  - Status: todo
  - Success criterion: Token transfers handle network errors gracefully with appropriate feedback

### H3: Staking Module (Priority #3)
- [ ] Create `staking/pools.js` - Pool creation and management
  - Status: todo
  - Success criterion: Admins can create and manage staking pools with customizable parameters
- [ ] Create `staking/stake.js` - Stake tokens into pools
  - Status: todo
  - Success criterion: Users can stake tokens in pools and track their stake
- [ ] Create `staking/unstake.js` - Unstake tokens with cooldown logic
  - Status: todo
  - Success criterion: Users can unstake tokens with proper cooldown period enforcement
- [ ] Create `staking/rewards.js` - Claim rewards from pools
  - Status: todo
  - Success criterion: Users can claim earned rewards from their staked positions
- [ ] Implement customizable APY calculation
  - Status: todo
  - Success criterion: Staking APY can be configured based on pool parameters and market conditions

---

## ⚙️ MEDIUM (Advanced Features)

### M1: Enhanced Security
- [ ] Implement rate limiting for API endpoints
  - Status: todo
  - Success criterion: API endpoints are protected against abuse with configurable rate limits
- [ ] Add audit logging for all transactions
  - Status: todo
  - Success criterion: All sensitive operations are logged with user, action, and timestamp
- [ ] Create security headers middleware
  - Status: todo
  - Success criterion: Application sets appropriate security headers (CSP, HSTS, etc.)
- [ ] Implement input validation and sanitization
  - Status: todo
  - Success criterion: All user input is validated and sanitized to prevent injection attacks
- [ ] Add IP whitelist capability for admin actions
  - Status: todo
  - Success criterion: Sensitive admin endpoints can be restricted to specific IP addresses

### M2: User Experience
- [ ] Create RESTful API documentation (openapi.yaml)
  - Status: todo
  - Success criterion: Complete OpenAPI specification is available for all endpoints
- [ ] Add request/response examples to documentation
  - Status: todo
  - Success criterion: API documentation includes realistic examples for all endpoints
- [ ] Implement error handling with helpful messages
  - Status: todo
  - Success criterion: Error responses provide clear, actionable information without exposing sensitive details
- [ ] Create health check endpoint `/health`
  - Status: todo
  - Success criterion: Health endpoint returns service status and dependencies
- [ ] Add user session management improvements
  - Status: todo
  - Success criterion: Session management includes refresh tokens and proper expiration

---

## 🐛 LOW (Nice to Have)

### L1: Testing & Documentation
- [ ] Write unit tests for all modules
  - Status: todo
  - Success criterion: Comprehensive test coverage (80%+) for all code modules
- [ ] Write integration tests for blockchain operations
  - Status: todo
  - Success criterion: Integration tests verify end-to-end functionality with blockchain interactions
- [ ] Create API testing guide
  - Status: todo
  - Success criterion: Guide available for developers to test API endpoints
- [ ] Update README with actual implementation status
  - Status: todo
  - Success criterion: README reflects current implementation state and usage
- [ ] Add example `.env` configuration comments
  - Status: todo
  - Success criterion: .env.example includes helpful comments explaining each variable

### L2: Operations & Monitoring
- [ ] Set up monitoring endpoints
  - Status: todo
  - Success criterion: Metrics endpoints available for system monitoring
- [ ] Create deployment scripts (Docker, systemd)
  - Status: todo
  - Success criterion: Automated deployment scripts available for production
- [ ] Add CI/CD pipeline configuration
  - Status: todo
  - Success criterion: CI/CD pipeline automates testing and deployment
- [ ] Write runbook for common issues
  - Status: todo
  - Success criterion: Runbook available for handling common operational issues
- [ ] Implement metrics collection
  - Status: todo
  - Success criterion: System collects and exposes key performance metrics

---

## Implementation Notes

### Tech Stack
- **Runtime**: Node.js 18+
- **Framework**: Express.js (lightweight, modular)
- **Database**: MongoDB (flexible schema for multi-chain data)
- **Auth**: JWT + 2FA tokens
- **Blockchain**: Direct API calls to Hive/TON nodes

### Security Requirements
- All token-moving actions require 2FA for Telegram-linked users
- Read-only operations (balances, NFTs) do not require 2FA
- Proper input validation on all endpoints
- Secrets stored in environment variables only

### Module Structure Pattern
```
src/
├── controllers/
│   ├── userController.js
│   ├── walletController.js
│   └── stakingController.js
├── services/
│   ├── walletService.js
│   ├── blockchainService.js
│   └── stakingService.js
├── models/
│   ├── User.js
│   ├── Wallet.js
│   └── StakingPool.js
├── middlewares/
│   ├── auth.js
│   ├── rateLimit.js
│   └── validation.js
└── routes/
    ├── userRoutes.js
    ├── walletRoutes.js
    └── stakingRoutes.js
```

### Task Progression Rules
- Tasks must be completed in priority order (Critical → High → Medium → Low)
- A task cannot be marked as complete until its success criterion is met
- Blocked tasks should include a note explaining the blocker
- Task status should be updated daily

### Repository Guidelines
- This task tracker should always reflect the current state of work
- When tasks are completed, mark them with [x]
- When work begins on a task, mark it with [>]
- If a task is blocked, mark it with [!]
- Add new tasks to this file as they are identified

*This task tracker was created on 2026-03-12.*