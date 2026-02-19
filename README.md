# BRNNode

## Project Overview
BRNNode is a modular, developer-friendly backend for blockchain-powered games and apps. It provides secure wallet management, staking, and token operations with seamless integration for both TON and Hive blockchains. The system enforces 2FA for sensitive actions (like transfers and staking) for Telegram-linked users, ensuring strong security while maintaining a smooth user experience. BRNNode is designed for rapid onboarding, extensibility, and easy integration with external apps and platforms.

## Key Features
- **Wallet module**: Secure, multi-chain wallet with 2FA enforcement for token-moving actions
- **Staking module**: Competitive, sustainable staking pools with customizable APY and reward logic
- **Hive Engine and TON integration**: Plug-and-play support for both blockchains
- **Modular architecture**: Easily extend or integrate new features and chains
- **Developer onboarding**: Clear documentation, examples, and contribution guidelines

## Quick Start
1. **Clone the repository:**
   ```sh
   git clone https://github.com/your-org/brnnode.git
   cd brnnode
   ```
2. **Install dependencies:**
   ```sh
   npm install
   # or
   yarn install
   ```
3. **Configure environment:**
   - Copy `.env.example` to `.env` and fill in required values (API keys, DB, etc.)
4. **Run the node:**
   ```sh
   npm start
   # or
   yarn start
   ```
5. **Access API docs and try example flows.**

## Documentation
- [Getting Started Guide](docs/getting-started.md)
- [Module Documentation](docs/modules.md)
- [API Reference](docs/api.md)
- [Contribution Guide](docs/contributing.md)
- [FAQ & Troubleshooting](docs/faq.md)
- [Architecture Diagrams](docs/diagrams.md)

## Architecture Diagram
See [Architecture Diagrams](docs/diagrams.md) for a system overview and module interactions.

## Community & Support
- Discord/Telegram: _Coming soon_
- Website: _Coming soon_
- For issues or feature requests, open a GitHub issue.

## License
MIT 

## Current State (2026-02-19 Audit)
- README standardized for operator clarity and execution planning.
- Detected stack: `unknown`.
- This section reflects the currently visible repository state and should be revised as implementation evolves.

## Product Requirements Document (PRD)

### Problem
- The project needs a clear, execution-ready specification that aligns implementation with expected outcomes.

### Product Summary
- `brn-node-gpt`: BRNNode is a modular, developer-friendly backend for blockchain-powered games and apps. It provides secure wallet management, staking, and token operations with seamless integration for both TON and Hive blockchains. The system enforces 2FA for sensitive actions (like transfers and staking) for Telegram-linked users, ensuring strong security while maintaining a smooth user experience. BRNNode is designed for rapid onboarding, extensibility, and easy integration with external apps and platforms.

### Goals
1. Clarify the product boundary, core use cases, and success criteria.
2. Ensure implementation tasks map directly to measurable outcomes.
3. Make handoff and maintenance possible without tribal knowledge.

### Primary Users
1. Project operator/owner managing roadmap and release decisions.
2. Developer/agent implementing and validating changes.

### Functional Requirements
1. Core workflow(s) are documented as inputs, processing steps, and outputs.
2. Runtime/deployment target is documented with required environment variables.
3. Error states and retries are defined for critical operations.

### Non-Functional Requirements
1. Observability: actionable logs and health/status visibility.
2. Reliability: deterministic behavior for critical paths.
3. Security: no secrets in repo, environment-based secret injection only.

### Milestones
1. M1: Baseline architecture and runbook clarity.
2. M2: Core workflow implementation and validation tests.
3. M3: Deployment hardening and operational checks.

### Definition of Done
1. A new operator can run and validate the project using README instructions.
2. Critical workflow tests pass consistently.
3. Deploy/restart/rollback steps are documented and verified.
