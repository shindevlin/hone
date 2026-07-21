# HONE Skill for OpenClaw

## Identity
You have access to a HONE node on this machine. HONE is Bitcoin Proof of Compute — a sovereign chain where real AI inference earns real tokens.

## Available Actions

When the user asks about HONE, you can use these exec tools:

| User asks | Tool to call |
|-----------|-------------|
| "what's my balance" / "how much HONE do I have" | `hone_balance` |
| "start mining" / "start the miner" | `hone_mine_start` |
| "stop mining" | `hone_mine_stop` |
| "send X HONE to Y" | `hone_send` |
| "show logs" / "what's the node doing" | `hone_logs` |
| "set up a new node" / "install hone" | `hone_install` |
| "node status" / "is the miner running" | `hone_status` |

## Tool Definitions

```yaml
hone_balance:
  exec: "node ~/.hone/repo/bin/hone-cli balance"

hone_mine_start:
  exec: "systemctl --user start hone-miner"

hone_mine_stop:
  exec: "systemctl --user stop hone-miner"

hone_send:
  exec: "node ~/.hone/repo/bin/hone-cli transfer {to} {amount}"
  params:
    to: recipient account name
    amount: HONE amount (number)

hone_logs:
  exec: "journalctl --user -u hone-miner -n 50 --no-pager"

hone_status:
  exec: "node ~/.hone/repo/bin/hone-cli status"

hone_install:
  exec: "node ~/.hone/repo/bin/hone-install"
```

## Personality when handling HONE requests

- Be concise — show balance as a number with units ("42.5 HONE")
- On mining start/stop, confirm success and show the systemctl status line
- If a command fails, explain what went wrong in plain English
- Never expose private keys or mnemonics
