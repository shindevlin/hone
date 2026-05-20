require('dotenv').config();
const TelegramBot = require('node-telegram-bot-api');
const axios = require('axios');
const express = require('express');

const BOT_TOKEN = process.env.ALERTBOT_TOKEN;
const API_URL = process.env.BTCPC_API_URL || 'https://btcpc.net';
const BOT_KEY = process.env.BOT_API_KEY;
const ALERT_API_KEY = process.env.ALERTBOT_API_KEY || 'changeme';
const PORT = process.env.PORT || 9900;

if (!BOT_TOKEN) { console.error('ALERTBOT_TOKEN required'); process.exit(1); }
if (!BOT_KEY) { console.error('BOT_API_KEY required'); process.exit(1); }

const bot = new TelegramBot(BOT_TOKEN, {
  polling: { interval: 2000, autoStart: true, params: { timeout: 10 } }
});

const api = axios.create({
  baseURL: `${API_URL}/api/bot`,
  headers: { 'x-bot-key': BOT_KEY },
  timeout: 15000
});

function fmt(n, d = 4) { return Number(n || 0).toFixed(d); }

// ── /start ──
bot.onText(/\/start/, (msg) => {
  bot.sendMessage(msg.chat.id, [
    `*BTCPC Wallet Bot*`,
    ``,
    `\`/link <username>\` — link your account`,
    `\`/verify <sig>:<recovery>\` — verify with posting key`,
    ``,
    `/balance /history /claim`,
    `/projects /mining /reward /unlink`,
  ].join('\n'), { parse_mode: 'Markdown' });
});

// ── /link ──
bot.onText(/\/link\s+(\S+)/, async (msg, match) => {
  try {
    const { data } = await api.post('/link', {
      username: match[1].toLowerCase(),
      telegramId: String(msg.from.id),
      telegramUsername: msg.from.username
    });
    bot.sendMessage(msg.chat.id, [
      `*Verify ownership of ${match[1]}*`,
      ``,
      `Sign: \`${data.challenge}\``,
      `CLI: \`node bin/btcpc-cli sign-challenge ${data.challenge}\``,
      `Then: \`/verify <signature>:<recovery>\``,
      `_Expires in ${data.expiresIn / 60} min_`,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /verify ──
bot.onText(/\/verify\s+(\S+)/, async (msg, match) => {
  const parts = match[1].split(':');
  if (parts.length !== 2 || !/^[0-9a-f]{128}$/i.test(parts[0])) {
    return bot.sendMessage(msg.chat.id, 'Format: `/verify <signature>:<recovery>`', { parse_mode: 'Markdown' });
  }
  try {
    const { data } = await api.post('/verify', {
      telegramId: String(msg.from.id),
      signature: parts[0],
      recovery: parseInt(parts[1], 10)
    });
    bot.sendMessage(msg.chat.id, `*${data.username}* linked! (tx: \`${data.tx_id}\`)`, { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /unlink ──
bot.onText(/\/unlink/, async (msg) => {
  try {
    await api.post('/unlink', { telegramId: String(msg.from.id) });
    bot.sendMessage(msg.chat.id, 'Unlinked.');
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /balance ──
bot.onText(/\/balance/, async (msg) => {
  try {
    const { data } = await api.get('/balance', { params: { telegramId: msg.from.id } });
    bot.sendMessage(msg.chat.id, [
      `*${data.username}* Wallet`,
      `Balance: \`${fmt(data.balance, 10)} BTCPC\``,
      `Proofs: ${data.proofCount} | Address: \`${data.address || 'none'}\``,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, err.response?.data?.error === 'Not linked' ? 'Link first: `/link <username>`' : `Error: ${err.message}`, { parse_mode: 'Markdown' });
  }
});

// ── /history ──
bot.onText(/\/history/, async (msg) => {
  try {
    const { data } = await api.get('/history', { params: { telegramId: msg.from.id } });
    if (!data.transactions.length) return bot.sendMessage(msg.chat.id, 'No transactions yet.');
    const lines = data.transactions.map(tx => `  ${tx.direction === 'in' ? '\u{2B06}' : '\u{2B07}'} ${tx.type}: ${fmt(tx.amount)} BTCPC`);
    bot.sendMessage(msg.chat.id, ['*Transactions*', '', ...lines].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /claim ──
bot.onText(/\/claim/, async (msg) => {
  try {
    const { data } = await api.post('/claim', { telegramId: String(msg.from.id) });
    bot.sendMessage(msg.chat.id, `Claimed 1 BTCPC. Balance: \`${fmt(data.balance, 10)}\``, { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /projects ──
bot.onText(/\/projects/, async (msg) => {
  try {
    const { data } = await api.get('/projects', { params: { telegramId: msg.from.id } });
    if (!data.projects.length) return bot.sendMessage(msg.chat.id, 'No projects.');
    const lines = data.projects.map(p => `  ${p.verified ? '\u{2705}' : '\u{23F3}'} *${p.name}* — ${fmt(p.balance, 10)} BTCPC | ${p.totalRequests} req`);
    bot.sendMessage(msg.chat.id, ['*Projects*', '', ...lines].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /mining ──
bot.onText(/\/mining/, async (msg) => {
  try {
    const { data } = await api.get('/proofs', { params: { telegramId: msg.from.id } });
    if (!data.proofs.length) return bot.sendMessage(msg.chat.id, 'No mining proofs yet.');
    const lines = data.proofs.slice(0, 5).map(p => `  Block ${p.block}: +${fmt(p.reward)} BTCPC | ${p.tokens} tokens`);
    bot.sendMessage(msg.chat.id, ['*Mining History*', '', ...lines].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── /reward ──
bot.onText(/\/reward/, async (msg) => {
  try {
    const { data } = await api.get('/reward');
    bot.sendMessage(msg.chat.id, `Reward: \`${fmt(data.reward, 10)} BTCPC\` (epoch ${data.epoch})`, { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.response?.data?.error || err.message}`);
  }
});

// ── Alert webhook ──
const app = express();
app.use(express.json());

app.post('/alert', async (req, res) => {
  const key = req.headers['x-api-key'];
  if (key !== ALERT_API_KEY) return res.status(401).json({ error: 'unauthorized' });

  const { project, severity = 'info', message, details } = req.body;
  if (!project || !message) return res.status(400).json({ error: 'project and message required' });

  try {
    const icons = { critical: '\u{1F6A8}', error: '\u{274C}', warn: '\u{26A0}\u{FE0F}', info: '\u{2139}\u{FE0F}', ok: '\u{2705}' };
    const text = `${icons[severity] || icons.info} *${project}*\n${message}`;

    // If earners list is provided, only DM users who actually earned this epoch.
    // Fall back to all linked users for non-mining alerts.
    const earners = details && Array.isArray(details.earners) && details.earners.length > 0
      ? details.earners
      : null;

    let users;
    if (earners) {
      const { data } = await api.get('/epoch-earners', { params: { earners: earners.join(',') } });
      users = data.users;
    } else {
      const { data } = await api.get('/linked-users');
      users = data.users;
    }

    for (const u of users) {
      bot.sendMessage(u.telegramId, text, { parse_mode: 'Markdown' }).catch(() => {});
    }
    res.json({ ok: true, sent: users.length, earners_only: !!earners });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/health', (_req, res) => res.json({ status: 'ok', service: 'btcpcwalletbot' }));

// ── Error handling ──
bot.on('polling_error', (err) => console.error('Polling error:', err.code || err.message));
process.on('uncaughtException', (err) => console.error('Uncaught:', err.message));
process.on('unhandledRejection', (reason) => console.error('Unhandled:', reason));

// ── Start ──
app.listen(PORT, () => console.log(`[btcpcwalletbot] Alert webhook on port ${PORT}`));
console.log('[btcpcwalletbot] @btcpcwalletbot is live (API mode)');
