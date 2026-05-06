# BTCPC Version Notes

Version: `2026-04-24-v7`

This file tracks the public surface revision that introduced the start-first
wizard, legal pages, and the controller/desktop alignment pass.

## Included in this version

- `/start` is the canonical first-stop route for humans and agents.
- `/agent` remains as a legacy alias.
- `/start.json` exposes the machine-readable onboarding manifest.
- Terms and Privacy pages are published on the website.
- The homepage, install page, app shell, controller page, README, roadmap, and
  whitepaper all point at the same start-first flow.
- The controller surface stays embedded in BTCPC with no BTCPC browser
  extension required.
- The roadmap now includes a dedicated start-first UX phase.
- `/start` now renders as a true step-by-step wizard with persistent progress,
  explicit completion states, and a machine-readable manifest at
  `/start.json`.
- `/setup` and `/app` now carry a start-first banner so the flow feels
  continuous instead of disconnected.
- `/setup` now reads the same `/start.json` manifest order, so the start
  sequence is mirrored instead of duplicated.
- `/app` now shows the current start step and done-when criteria from the
  same manifest.
- The shared start-state helper now powers `/start`, `/setup`, and `/app`
  manifest loading and rendering.
- `/start` now begins with a username availability check so the on-chain
  identity is chosen before account creation.
- `/setup` pre-fills the saved username when one has already been selected in
  `/start`.
- The docs now split into a plain-language start guide and a technical deep
  dive, and the contribution guide now explicitly asks for better mechanisms
  back as pull requests.

## Notes

- This note is intentionally short and public-facing.
- It should be updated whenever the site surface or onboarding flow changes in
  a user-visible way.
- Keep the version string synchronized with the footers on the main public
  pages.
