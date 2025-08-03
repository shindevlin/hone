# Architecture Diagrams & Visuals

## System Overview
```mermaid
graph TD
  User["User"]
  Telegram["Telegram"]
  Hive["Hive Blockchain"]
  TON["TON Blockchain"]
  UnifiedAccount["UnifiedAccountModule"]
  Wallet["Wallet Module"]
  Staking["Staking Module"]
  HiveEngine["HiveEngineModule"]
  TONModule["TONModule"]

  User --> UnifiedAccount
  UnifiedAccount --> Wallet
  UnifiedAccount --> Staking
  UnifiedAccount --> Telegram
  Wallet --> HiveEngine
  Wallet --> TONModule
  Staking --> HiveEngine
  Staking --> TONModule
  HiveEngine --> Hive
  TONModule --> TON
```

## Module Interactions
- **UnifiedAccountModule** enforces 2FA for token-moving actions and manages user authentication.
- **Wallet Module** handles balances and transfers, integrating with Hive and TON via respective modules.
- **Staking Module** manages pools, staking, and rewards, also integrating with blockchains. 