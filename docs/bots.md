# HONE Telegram Bots

## Architecture

Bots are **thin HTTP clients**. They have no database access — all data comes from the HONE API at `/api/bot/*`, authenticated by `x-bot-key` header.

```
Telegram → Bot (polling) → HONE API (/api/bot/*) → MongoDB
```

## Active Bots

| Bot | Username | Repo | Purpose |
|-----|----------|------|---------|
| HONE Bot | @honebot | `~/repos/honebot/` | Inference, network, mining, linking |
| Wallet Bot | @honewalletbot | `~/repos/honewalletbot/` | Balance, history, claim, projects, alerts |

## Token Management

- Tokens are in each bot's `.env` file (NEVER in git)
- `.env` files are gitignored
- If tokens get compromised, revoke via @BotFather and paste new tokens directly into `.env`
- **NEVER share tokens in chat, commits, or any text that could be logged**

## Starting Bots

```bash
# IMPORTANT: Kill all zombies first
for pid in $(pgrep -x node); do
  cwd=$(readlink /proc/$pid/cwd 2>/dev/null)
  if echo "$cwd" | grep -qE "honebot|honewalletbot|telegram-bot|alertbot"; then
    kill -9 $pid
  fi
done

# Wait for Telegram session release
sleep 35

# Start exactly one of each
cd ~/repos/honebot && node index.js > /tmp/honebot.log 2>&1 &
sleep 5
cd ~/repos/honewalletbot && node index.js > /tmp/honewalletbot.log 2>&1 &
```

## Zombie Prevention

409 "Conflict" errors mean multiple processes are polling the same token.

```bash
# Find zombies
for pid in $(pgrep -x node); do
  cwd=$(readlink /proc/$pid/cwd 2>/dev/null)
  if echo "$cwd" | grep -qE "honebot|honewalletbot|telegram-bot|alertbot"; then
    echo "PID $pid: $cwd"
  fi
done
```

## API Endpoints Used

All at `/api/bot/*`, require `x-bot-key` header:

| Endpoint | Method | Purpose |
|----------|--------|---------|
| /user | GET | Get user by telegramId |
| /link | POST | Start verification challenge |
| /verify | POST | Verify posting key signature |
| /unlink | POST | Unlink telegram |
| /balance | GET | Balance, staked, proofs, dreams |
| /history | GET | Transaction history |
| /claim | POST | Faucet claim |
| /mining | GET | Mining stats |
| /proofs | GET | Mining proofs |
| /dreams | GET | Genesis dreams |
| /node | GET | Node status |
| /network | GET | Network overview |
| /epoch | GET | Current epoch |
| /reward | GET | Block reward |
| /pricing | GET | Inference pricing |
| /models | GET | Network models |
| /peers | GET | Peer list |
| /peers/register | POST | Register peer |
| /peers/heartbeat | POST | Peer heartbeat |
| /projects | GET | User projects |
| /inference | POST | Submit inference |
| /inference/:jobId | GET | Poll inference job |
| /linked-users | GET | All linked users (for alerts) |

## Linking Flow

1. User sends `/link <username>` on either bot
2. Bot calls `POST /api/bot/link` → gets challenge
3. User signs challenge with posting key: `node bin/hone-cli sign-challenge <challenge>`
4. User sends `/verify <signature>:<recovery>`
5. Bot calls `POST /api/bot/verify` → posting key signature verified → on-chain tx recorded

## Other Telegram Bots (NOT HONE)

| Bot | Project | Location |
|-----|---------|----------|
| Bullship Bot | bullship | Docker: `bullship-telegram-bot-1` |
| nsfwotica Bot | nsfwotica | `nsfwotica/telegram/bot.js` |
| brutus11 Bot | brutus11 | `brutus11/telegram-bot/bot.js` |
