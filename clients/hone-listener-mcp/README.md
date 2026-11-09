# hone-listener-mcp

An MCP server that lets an AI assistant (Claude Desktop, Claude Code, or any MCP client) set up the
Ornigo listener on the user's computer and answer questions like "is the cardinal back?".

Docs for people: https://honemesh.net/listener/ai

## Tools

| tool | what it does |
|---|---|
| `install_listener` `{confirm?, ref?}` | Detects the OS, downloads `version.json` + `version.json.sig` and the installer from `https://honemesh.net/app/listener/` only, checks the ed25519 signature against the pinned HONE release key and the installer's sha256. Without `confirm` it only reports the checks. With `confirm: true` (after the user agrees) it verifies again and runs the exact verified bytes, stdin closed. |
| `listener_status` | Local service state (Windows service / scheduled task, launchd, systemd) plus every listener on the account: "Listening now" or last report. |
| `my_birds` `{since?}` | Species heard by the user's own devices: names, reports, places, first/last heard, rare flag. |
| `is_back` `{name}` | Was this bird heard in the last 24 h, and how long was it away before that? |

No tool accepts a URL. Nothing unverified is ever run.

## Install

Node 18.17 or newer.

```json
{
  "mcpServers": {
    "hone-listener": {
      "command": "npx",
      "args": ["-y", "https://honemesh.net/app/mcp/hone-listener-mcp-0.1.1.tgz"],
      "env": { "HONE_TOKEN": "w_... (from https://honemesh.net/me, Make an AI key)" }
    }
  }
}
```

Claude Code: `claude mcp add hone-listener -e HONE_TOKEN=w_... -- npx -y https://honemesh.net/app/mcp/hone-listener-mcp-0.1.1.tgz`

`HONE_TOKEN` is optional for installing; it is needed for `my_birds`, `is_back` and account status.
It may also be read from `~/.hone/listener-token`. Only read-only web keys (`w_...`) are accepted.

Remote, read-only (no installs): `node server.mjs --http 9461` serves MCP over HTTP (JSON responses,
stateless). The public one is `https://honemesh.net/listener/mcp`; send `Authorization: Bearer w_...`.

## Release contract (for whoever publishes the listener)

Published at `https://honemesh.net/app/listener/`:

- `version.json` (`local.status_port_file` per OS is read first by `listener_status`, then `local.status_port`): `{"name": "hone-listener", "version": "...", "files": {"install.sh": {"sha256": "..."}, "install.ps1": {"sha256": "..."}, ...}}`.
  Optional: `"store_url"` (Microsoft Store listing, shown on /listener), `"local": {"windows_service", "windows_task", "launchd_label", "systemd_unit", "status_port"}` (used by `listener_status`).
- `version.json.sig`: base64 (or hex) ed25519 signature over the exact bytes of `version.json`,
  by the Ornigo (hone-listener) release key `27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d`.
  `scripts/sign-listener-release.mjs <dir> --version X.Y.Z` writes both.
- Installers get `HONE_REF` (invite code, may be empty), `HONE_VERIFIED_MANIFEST` (path to the verified
  `version.json`, so the installer can check every file it downloads against it), `HONE_INSTALL_SOURCE=mcp`
  and `HONE_NONINTERACTIVE=1` (stdin is closed: finish sign-in and camera choice in a browser page).
