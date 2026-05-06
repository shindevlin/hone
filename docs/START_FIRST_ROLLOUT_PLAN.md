# BTCPC Start-First Rollout Plan

Version: `2026-04-24-v7`
Status: active planning note

This note records the next public UX pass for BTCPC. The goal is to make the
chain easy to approach for both humans and AI agents without requiring them to
already understand the repo structure or the policy model.

## Goals

- Make `/start` the canonical first-stop route for humans and agents.
- Turn the current multi-page install flow into one obvious onboarding path.
- Make controller mode turnkey inside the existing BTCPC web and desktop app.
- Add user-visible Terms and Privacy policy pages.
- Keep the desktop, browser, and agent flows aligned so the same guidance works
  everywhere.
- Version the rollout so docs, roadmap, and site copy stay in sync.

## Workstreams

### 1. Canonical entrypoint

- `/start` becomes the public front door.
- `/agent` remains as a legacy alias for compatibility.
- Homepage, install page, README, and whitepaper point to `/start`.
- The desktop app exposes the same start guide in its shell.

### 2. Guided onboarding

- A first-run path tells the user exactly what to do:
  - choose an on-chain username and check availability
  - create or import an account
  - save the wallet export to disk
  - open inference
  - choose node roles
  - choose controller policy if desired
- `/start` should present one obvious next action at a time, not a wall of
  options.
- The same flow should expose a machine-readable manifest for agents and
  automation.

### 3. Controller policy

- Privy remains the embedded controller layer for supported external-chain
  signing.
- BTCPC native spending keeps local active-key signing available.
- External controller mode can disable active-key sends when the user opts in.
- Secondary approval stays a separate outside-wallet lane.

### 4. Legal surfaces

- Publish Terms and Privacy Policy pages on the website.
- Link them from the homepage, install page, controller page, and app shell.
- Keep the language clear about wallet handling, local storage, and third-party
  providers.

### 5. Mobile approval

- Make controller approval work cleanly on phones.
- Prefer QR / deep-link / one-tap confirmation flows.
- Return the user to BTCPC automatically after signing.

### 6. Versioning and notes

- Maintain a dated version note for the public onboarding surface.
- Update README, roadmap, whitepaper, and site copy together.
- Treat each public change as a versioned release note, not an ad hoc edit.

## Initial acceptance criteria

- A new user can open `/start` and understand what to do within one minute.
- A first-time agent can follow the page and complete install deterministically.
- A user can find Terms and Privacy from every major public surface.
- Desktop and browser both expose the controller flow without a BTCPC extension.

## Change log

- `2026-04-24-v1` - Initial start-first rollout plan drafted.
- `2026-04-24-v2` - `/start` upgraded into a stepwise wizard with a
  persistent machine-readable manifest and completion state.
- `2026-04-24-v3` - `/setup` and `/app` get start-first banners so the
  onboarding flow feels continuous across surfaces.
- `2026-04-24-v4` - `/setup` mirrors the `/start.json` manifest order to
  reduce duplicated onboarding logic.
- `2026-04-24-v5` - `/app` displays the current start step and done criteria
  from the same manifest for a continuous browser flow.
- `2026-04-24-v6` - Shared start-state helper powers manifest loading and
  render state across `/start`, `/setup`, and `/app`.
- `2026-04-24-v7` - `choose-username` becomes the first wizard step, and
  `/setup` pre-fills the saved name from `/start`.
