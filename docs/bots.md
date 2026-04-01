# BTCPC Telegram Bots

## Active Bots

| Bot | Token Prefix | Username | Port | Purpose | Location |
|-----|-------------|----------|------|---------|----------|
| BTCPC Bot | `8754439991` | @btcpcbot | 3003 | Inference, network, mining, linking | `btcpc/telegram-bot/index.js` |
| Wallet Bot | `8756458842` | @btcpcwalletbot | 9900 | Balance, history, claim, projects, alerts | `alertbot/index.js` |

## Token Locations

- `btcpc/telegram-bot/.env` — `BTCPC_BOT_TOKEN`
- `alertbot/.env` — `ALERTBOT_TOKEN`

## Shared State

Both bots connect to the same MongoDB (`btcpc` database) and share the same User model.
A user linked on one bot is immediately visible to the other.

## Known Process Issues

**CRITICAL: Only ONE instance of each bot can poll Telegram at a time.**

If you get `409 Conflict: terminated by other getUpdates request`, there are zombie processes:

```bash
# Find zombies
pgrep -a node | grep -E "btcpc/telegram-bot|alertbot"

# Kill all
pkill -9 -f "btcpc/telegram-bot/index"
pkill -9 -f "alertbot/index"

# Wait 30s for Telegram session release
sleep 30

# Start exactly one of each
cd ~/repos/btcpc/telegram-bot && node index.js > /tmp/btcpcbot.log 2>&1 &
cd ~/repos/alertbot && node index.js > /tmp/btcpcwalletbot.log 2>&1 &
```

## Other Telegram Bots (NOT BTCPC)

These use DIFFERENT tokens — no conflict with BTCPC bots:

| Bot | Token Prefix | Project | Location |
|-----|-------------|---------|----------|
| Bullship Bot | `7830932174` | bullship | Docker: `bullship-telegram-bot-1` |
| nsfwotica Bot | (separate) | nsfwotica | `nsfwotica/telegram/bot.js` |
| ursOS Bot | (separate) | ursOS | Docker: `urs-node-vault-app` |

## Linking Flow

1. User sends `/link <username>` on either bot
2. Bot returns challenge: `BTCPC-VERIFY-xxxxxxxx`
3. User replies `/verify BTCPC-VERIFY-xxxxxxxx`
4. Bot records verification on-chain and links Telegram ID to BTCPC account
