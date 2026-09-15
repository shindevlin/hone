# HONE Post-Launch Backlog

**Purpose:** everything deliberately parked so we converge on launch. Nothing here
is abandoned — it is *sequenced after* public testnet is green. Re-entry criteria
at the bottom.

**Rule:** do not start any item below while the Go-Live Checklist
(`docs/GO_LIVE_CHECKLIST.md`) has open boxes. Widening is what kept launch weeks away.

## Parked tracks

1. **Need-based model routing** — branch `feat/model-router` (scaffold only, nothing
   committed). Difficulty classifier → tier (small / mid / frontier) → concrete model
   requirement feeding `hone-orchestrator` `RuntimeRequirements.models`. Routes by task
   *need*, not the requested model. Deterministic so the decision can be attested.
   Reusable by Ferryman workers.

2. **Ferryman ↔ inference integration** — generalize the working prototype
   `C:\NATV-ops\omniroute_worker.py` (leases from hub `127.0.0.1:8796`, calls a gateway
   for inference) into a reusable Ferryman worker; wire the router from (1). Any gateway
   stays an *optional backend*, never on the critical path.

3. **OmniRoute hardening** *(only if adopted)* — it is running on Beastly but:
   bind `:20128` to `127.0.0.1` (currently `0.0.0.0`, LAN-exposed), pin the version,
   back up `~/.omniroute/.env` `STORAGE_ENCRYPTION_KEY`, keep tunnels off
   (`LIVE_WS_HOST`, `TAILSCALE_AUTHKEY`, ngrok). Do **not** depend on impersonated free
   tiers — Cloudflare `1010` bans already observed against opencode.ai.

4. **Standalone wallet-creation app (`hone-seal`)** — shipped on `feat/sealed-backup`
   (commit `967512a0`). Follow-ups: mac/linux native builds, signed releases, optional
   GUI wrapper.

5. **Ferryman ops polish** — Windows Task Scheduler auto-boot of WSL + hub after reboot;
   hub is ~1 docs commit behind `estejosh/ferryman` main; remaining friction items.

6. **"seccheck" git-security / injection-sniffer tool** — to be built in Codex.

7. **Dispatch VIS → shuffler wiring** — HONE crypto-critical; human-gated; Codex/OpenClaw.

8. **HONE landing page + SDK docs.**

## Re-entry criteria

Resume the above **only** when all are true:
- Every box in `docs/GO_LIVE_CHECKLIST.md` is green under retry/quorum checks.
- Public testnet has been stable (no localhost chain-truth; ≥2 real nodes) for a sustained window.
- Mainnet/genesis remains a separate, human-only decision and is **not** unblocked by this list.
