#!/usr/bin/env bash
# First-time HONE node setup on a new machine.
# Run as root or with sudo.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/shindevlin/hone/stable/scripts/install.sh | sudo bash

set -euo pipefail

GITHUB_REPO="shindevlin/hone"
RELEASE_TAG="node-v1.1.0"
ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) SUFFIX="aarch64-linux" ;;
  *)             SUFFIX="x86_64-linux"  ;;
esac
ASSET_NODE="hone-node-${SUFFIX}"
ASSET_CLI="hone-${SUFFIX}"
BINARY_NODE="/usr/local/bin/hone-node"
BINARY_CLI="/usr/local/bin/hone"
RELEASE_BASE="https://github.com/${GITHUB_REPO}/releases/download/${RELEASE_TAG}"
SCRIPTS_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable/scripts"
RAW_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable"

# ── License: UFL 3.4, Operational Scope: Decentralized ─────────────────────
# HONE does not run until its license is accepted (UFL Section 9), so ask
# before installing anything. Typing "I accept" (or HONE_ACCEPT_LICENSE=UFL-3.4-D
# for unattended installs) is the act of acceptance. Declining installs nothing.
# Keep the summary in step with hone_license::summary() (a Rust test checks it).
hone_license_summary() {
cat <<'HONE_LICENSE_SUMMARY'
HONE is licensed under the Usufruct License (UFL) 3.4, Operational Scope: Decentralized (LicenseRef-UFL-3.4-D).
Full text: https://github.com/shindevlin/hone/blob/main/LICENSE

Summary (the license text governs):
  * Use is free at any scale and nothing is paid. Redistribution is reserved,
    except that forks of HONE as network software may be distributed if they
    credit HONE and keep the license's canonical-source notice (Section 2A).
  * Disputes: every dispute arising from your use of HONE is decided only by
    online arbitration under the ICC Rules, before a single arbitrator seated
    in California, on an individual basis and not as a class or collective
    action. The arbitrator applies the UNIDROIT Principles, then California
    law. Each side bears its own costs and fees unless a claim or defense is
    frivolous or in bad faith.
  * Liability cap: each party's total liability to the other for all claims
    is limited, to the extent the law allows, to one US dollar in value,
    paid in money or in HONE at the paying party's choice.
  * Forks and bypassers are at their own risk: running HONE with this step
    bypassed, removed, or forged breaches the license and makes that copy a
    modified version, and the Licensor then owes you nothing. A modified
    version, fork, or reimplementation is used entirely at your own risk.
  * HONE is provided as is, without warranty.
  * Nothing here limits liability or waives rights that the law does not
    allow to be limited or waived, or stops either side seeking a court
    injunction against conduct Section 2 reserves or IP infringement.
HONE_LICENSE_SUMMARY
}
hone_license_accepted_in() { # DATA_DIR
  grep -q '"license": "UFL"' "$1/license-acceptance.json" 2>/dev/null &&
  grep -q '"version": "3.4"' "$1/license-acceptance.json" 2>/dev/null &&
  grep -q '"scope": "Decentralized"' "$1/license-acceptance.json" 2>/dev/null
}
hone_license_record() { # DATA_DIR — license, version, scope, UTC time, installer name; local only
  mkdir -p "$1" && cat > "$1/license-acceptance.json" <<HONE_LICENSE_RECORD
{
  "license": "UFL",
  "version": "3.4",
  "scope": "Decentralized",
  "accepted_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "binary": "hone-installer"
}
HONE_LICENSE_RECORD
}
hone_license_gate() { # DATA_DIR — returns 0 if accepted (now or before)
  if hone_license_accepted_in "$1"; then return 0; fi
  if [ -n "${HONE_ACCEPT_LICENSE:-}" ]; then
    if [ "$HONE_ACCEPT_LICENSE" = "UFL-3.4-D" ]; then
      echo "Accepted UFL 3.4, Operational Scope: Decentralized via HONE_ACCEPT_LICENSE=UFL-3.4-D."
      return 0
    fi
    echo "HONE_ACCEPT_LICENSE is set to \"$HONE_ACCEPT_LICENSE\". The only value that accepts HONE's license is UFL-3.4-D. Nothing was installed." >&2
    return 1
  fi
  echo ""
  hone_license_summary
  if ( exec </dev/tty ) 2>/dev/null; then
    printf 'Type "I accept" to accept and install, or anything else to stop: '
    HONE_LICENSE_ANSWER=""
    read -r HONE_LICENSE_ANSWER </dev/tty || true
    if [ "$HONE_LICENSE_ANSWER" = "I accept" ]; then return 0; fi
    echo "License not accepted. Nothing was installed."
    return 1
  fi
  echo "No terminal to ask on. To accept the license above and install, run again with"
  echo "HONE_ACCEPT_LICENSE=UFL-3.4-D set. Setting it is the act of acceptance. Nothing was installed."
  return 1
}
if ! hone_license_gate /var/lib/hone; then
  exit 3
fi

echo "==> Installing HONE node and CLI ${RELEASE_TAG} (${SUFFIX})"

# ── Download node binary ──────────────────────────────────────────────────────
echo "==> Downloading node binary"
curl -fsSL "${RELEASE_BASE}/${ASSET_NODE}" -o "$BINARY_NODE"
chmod +x "$BINARY_NODE"
echo "==> Installed: $BINARY_NODE"

# ── Download CLI binary ───────────────────────────────────────────────────────
echo "==> Downloading hone CLI"
curl -fsSL "${RELEASE_BASE}/${ASSET_CLI}" -o "$BINARY_CLI"
chmod +x "$BINARY_CLI"
echo "==> Installed: $BINARY_CLI"

# ── Mainnet data directory + user ────────────────────────────────────────────
mkdir -p /var/lib/hone
id hone &>/dev/null || useradd --system --no-create-home --home /var/lib/hone hone
chown hone:hone /var/lib/hone

# ── Testnet data directory + genesis ────────────────────────────────────────
mkdir -p /var/lib/hone-testnet
chown hone:hone /var/lib/hone-testnet
echo "==> Downloading testnet genesis"
curl -fsSL "${RAW_BASE}/rust/hone-node/testnet-genesis.json" \
    -o /var/lib/hone-testnet/genesis.json
chown hone:hone /var/lib/hone-testnet/genesis.json

# ── License acceptance record for both services' data directories ──────────
for d in /var/lib/hone /var/lib/hone-testnet; do
  hone_license_accepted_in "$d" || hone_license_record "$d"
  chown hone:hone "$d/license-acceptance.json"
done

# ── Systemd units ─────────────────────────────────────────────────────────────
echo "==> Installing systemd units"
curl -fsSL "${SCRIPTS_BASE}/hone-node.service"     -o /etc/systemd/system/hone-node.service
curl -fsSL "${SCRIPTS_BASE}/hone-testnet.service"  -o /etc/systemd/system/hone-testnet.service
curl -fsSL "${SCRIPTS_BASE}/hone-update.service"   -o /etc/systemd/system/hone-update.service
curl -fsSL "${SCRIPTS_BASE}/hone-update.timer"     -o /etc/systemd/system/hone-update.timer
systemctl daemon-reload

# ── Enable auto-update timer ──────────────────────────────────────────────────
systemctl enable --now hone-update.timer
echo "==> Auto-updates enabled"

echo ""
echo "==> Done. Next steps:"
echo ""
echo "  1. Create your account and log in:"
echo "       hone account create yourname"
echo "       hone login"
echo ""
echo "  2. Set your account in both service files:"
echo "       nano /etc/systemd/system/hone-node.service"
echo "       nano /etc/systemd/system/hone-testnet.service"
echo "     Required env vars in each:"
echo "       HONE_ACCOUNT   — your account name"
echo "       HONE_NODE_ID   — unique node label"
echo ""
echo "  3. Start both mainnet and testnet:"
echo "       systemctl enable --now hone-node hone-testnet"
echo ""
echo "  4. Check status:"
echo "       systemctl status hone-node hone-testnet"
echo "       curl http://localhost:4242/api/node/info   # mainnet"
echo "       curl http://localhost:4343/api/node/info   # testnet"
echo ""
