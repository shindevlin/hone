# Archived: Node.js prototype (v3.x)

The original Node.js/Express implementation of HONE (`src/`, its jest `tests/`, the
`bin/` helpers, the Electron `desktop/` shell and the root `package.json` it used).
It is not running and is not maintained (see `src/DEPRECATED.md`). The active node is
`rust/hone-node`.

Moved here on 2026-10-23 in the formal archive pass that `src/DEPRECATED.md` called for.
`package.json` / `package-lock.json` are kept as `*.archived` so dependency scanners
do not report vulnerabilities for code that never runs. To revive it, rename them back.

The root `package.json` now only carries the Capacitor tooling for the Android app in
`android/`.
