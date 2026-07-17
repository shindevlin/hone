# Verasens Institutional Dashboard

B2B web service implementing PRD Phase 1.4 ("Build the dashboard"). Source
lives at `rust/verasens-dashboard/` (crate name `verasens-dashboard`); this
directory is the PRD's suggested `verasens/dashboard/` location and holds
docs/env config for the service.

See `docs/PLATFORM_PRD.md` Phase 1.4 for the full DECISION RECORD this
implements (auth/session model, Stripe billing model, rate plan, settlement
seam). This service does not implement the USD->BTCPC settlement bridge
(separate PRD item) or SSO/SAML for the Enterprise tier (deferred, see
follow-ups below).

## Run

```bash
cp verasens/dashboard/.env.example verasens/dashboard/.env
# edit .env — at minimum set DATABASE_URL and VERASENS_AGG_URL for a real run;
# leave DATABASE_URL/STRIPE_SECRET_KEY unset to run against in-memory/mock
# backends for local demo purposes only.
cd rust && cargo run -p verasens-dashboard
```

## Test

```bash
cd rust
cargo test -p verasens-dashboard                     # unit + mock-backed integration tests
DATABASE_URL=postgres://... cargo test -p verasens-dashboard --test postgres_integration  # real-Postgres tests
```

`tests/postgres_integration.rs` skips (prints a notice, does not fail) when
`DATABASE_URL` is unset.

## API surface (`/api/dashboard/*`)

- `POST /signup` — create org + owner member.
- `POST /login` — email+password; returns a session cookie, or a
  `totp_required` challenge if the member has TOTP enabled.
- `POST /totp/enroll`, `POST /totp/verify` — TOTP 2FA setup/verification.
- `POST /logout` — revokes the session.
- `POST /keys`, `GET /keys`, `POST /keys/:id/revoke` — API key management
  (owner/admin only).
- `GET /query` — the query builder: forwards to the aggregation service's
  `GET /api/verasens/query`, renders grouped results + provenance, meters
  the query if billable. Accepts either a session cookie or
  `Authorization: Bearer btcpc_live_...`.
- `GET /usage` — usage history.

## What's built vs. stubbed

Built and tested against real logic: password hashing (argon2id), TOTP 2FA
enrollment/verification, HttpOnly/Secure/SameSite=Lax session cookies with
idle+absolute expiry backed by a server-side store, API key issuance
(shown once, salted-hash storage, scoped, revocable), the query builder
route, and the metering pipeline (usage_event ledger write BEFORE Stripe
meter report, same-transaction settlement_intent rows, idempotent
reporting via `usage_event.id` as the Stripe idempotency key).

Stubbed/mocked, explicitly:
- **Stripe**: `HttpStripeClient` implements the real API calls (customer
  create, meter event report) but is not exercised against a live Stripe
  account in this build's tests — `MockStripeClient` is what the test
  suite runs against. Set `STRIPE_SECRET_KEY` to switch the running
  service to live Stripe; no code changes needed.
- **Settlement bridge**: `settlement_intent` rows are written (the seam
  the PRD specifies) but nothing drains them — that's the separate
  USD->BTCPC settlement bridge PRD item.
- **SSO/SAML** for the Enterprise tier: not implemented; Enterprise orgs
  currently fall back to the same password+TOTP path as Starter/Growth.
- **"Heavy query" units multiplier**: metering charges a flat 1 unit per
  billable query (`billing::DEFAULT_UNITS_PER_QUERY`); the PRD's
  large-geo/long-window/export multiplier table is not implemented.
- **Nightly reconciliation scheduler**: `billing::reconcile_unreported_usage`
  is implemented and tested but nothing cron-schedules it yet.
