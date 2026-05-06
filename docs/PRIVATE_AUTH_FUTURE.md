# Private Authorization Future Notes

This page documents the staged private-authorization system that exists in code but remains disabled by default.

## Status

- Runtime feature flag: `BTCPC_PRIVATE_AUTH_ENABLED`
- Default state: off
- Purpose: make the future approval flow discoverable in code and docs before it is ever turned on

## Chain Comparison

| Chain | Approval Form | Existing Wallet Friendly | Privacy Goal | Current Status |
|---|---|---:|---:|---|
| Bitcoin | Signed challenge | Yes | Low now, higher later | Staged |
| Lightning | Invoice payment receipt | Yes | Medium now, higher later | Staged |
| EVM | Wallet signature | Yes | Low now | Bridge support |
| Solana | Wallet signature | Yes | Low now | Bridge support |
| TON | Wallet signature | Yes | Low now | Bridge support |
| zkVM | Portable proof receipt | Not wallet-specific | High | Staged adapter |

## Enablement Checklist

This is the checklist the branch expects to be satisfied before a future rollout can turn the feature flag on.

1. Bitcoin approval previews render correctly.
2. Lightning invoice receipts are available from the configured provider.
3. zkVM verifier backend is reachable and returns valid receipts.
4. Policy read/write flows are reviewed.
5. Transfer request and transfer verify endpoints are reviewed.
6. Telegram webapp preview surfaces match the route summaries.
7. Bot API preview surfaces match the route summaries.
8. Logging remains redacted for approval identifiers.
9. Feature flag remains off until release approval.

## Route Surface

The staged route set is documented in the runtime branch and should remain visible in the code:

- `/api/wallet/private-auth`
- `/api/wallet/private-auth/preview`
- `/api/wallet/private-auth/routes`
- `/api/wallet/private-auth/policy`
- `/api/wallet/private-auth/enroll/request`
- `/api/wallet/private-auth/enroll/verify`
- `/api/wallet/private-auth/transfer/request`
- `/api/wallet/private-auth/transfer/verify`

The bot and explorer surfaces expose the same staged concepts under their own prefixes.

## Why it exists

The goal is to keep the future private-auth stack visible and auditable without turning it on early. That lets the code show:

- what the approval flow will look like,
- which chains can be used as policy anchors,
- and what must be true before the future update can be released.

