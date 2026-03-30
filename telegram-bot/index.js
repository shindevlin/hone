require('dotenv').config({ path: require('path').resolve(__dirname, '.env') });
const TelegramBot = require('node-telegram-bot-api');
const mongoose = require('mongoose');

const BOT_TOKEN = process.env.BTCPC_BOT_TOKEN;
const MONGODB_URI = process.env.MONGODB_URI || 'mongodb://root:example@localhost:27017/btcpc?authSource=admin';

if (!BOT_TOKEN) {
  console.error('BTCPC_BOT_TOKEN required');
  process.exit(1);
}

// ── Load Mongoose models ──
const User = require('../src/models/User');
const Wallet = require('../src/models/Wallet');
const Node = require('../src/models/Node');
const Epoch = require('../src/models/Epoch');
const WorkProof = require('../src/models/WorkProof');
const MiningProof = require('../src/models/MiningProof');
const GenesisDream = require('../src/models/GenesisDream');
const Transaction = require('../src/models/Transaction');
const PeerRegistry = require('../src/models/PeerRegistry');
const { getBlockReward } = require('../src/services/emissionSchedule');

const bot = new TelegramBot(BOT_TOKEN, { polling: true });

// ── Helpers ──
function fmt(n, decimals = 4) {
  return Number(n || 0).toFixed(decimals);
}

function esc(s) {
  return String(s).replace(/[_*[\]()~`>#+\-=|{}.!]/g, '\\$&');
}

async function getLinkedUser(telegramId) {
  return User.findOne({ telegramId: String(telegramId) });
}

// ── /start ──
bot.onText(/\/start/, (msg) => {
  bot.sendMessage(msg.chat.id, [
    `\u{26D3} *BTCPC Node Bot*`,
    ``,
    `Link your account: \`/link <username>\``,
    ``,
    `*Commands:*`,
    `/balance — BTCPC balance & staked`,
    `/mining — mining stats & recent epochs`,
    `/epoch — current epoch info`,
    `/network — network overview`,
    `/proofs — your mining proofs (soulbound)`,
    `/dreams — your genesis dreams (NFTs)`,
    `/node — your node status`,
    `/history — recent transactions`,
    `/reward — current block reward`,
    `/peers — list registered P2P peers`,
    `/register <ws://ip:port> — register your node for peer discovery`,
    `/unlink — unlink Telegram account`,
  ].join('\n'), { parse_mode: 'Markdown' });
});

// ── /link <username> ──
bot.onText(/\/link\s+(\S+)/, async (msg, match) => {
  const chatId = msg.chat.id;
  const tgId = String(msg.from.id);
  const username = match[1].toLowerCase();

  try {
    const existing = await User.findOne({ telegramId: tgId });
    if (existing) {
      return bot.sendMessage(chatId, `Already linked to *${esc(existing.username)}*\\. Use /unlink first\\.`, { parse_mode: 'MarkdownV2' });
    }

    const user = await User.findOne({ username });
    if (!user) {
      return bot.sendMessage(chatId, `Account \`${username}\` not found on BTCPC chain.`, { parse_mode: 'Markdown' });
    }

    if (user.telegramId && user.telegramId !== tgId) {
      return bot.sendMessage(chatId, `That account is already linked to another Telegram user.`);
    }

    user.telegramId = tgId;
    user.telegramUsername = msg.from.username || null;
    await user.save();

    bot.sendMessage(chatId, `\u{2705} Linked to *${esc(username)}*`, { parse_mode: 'MarkdownV2' });
  } catch (err) {
    console.error('Link error:', err.message);
    bot.sendMessage(chatId, `Error: ${err.message}`);
  }
});

// ── /unlink ──
bot.onText(/\/unlink/, async (msg) => {
  const tgId = String(msg.from.id);
  try {
    const user = await User.findOne({ telegramId: tgId });
    if (!user) return bot.sendMessage(msg.chat.id, 'No account linked.');
    user.telegramId = null;
    user.telegramUsername = null;
    await user.save();
    bot.sendMessage(msg.chat.id, '\u{1F517} Unlinked.');
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /balance ──
bot.onText(/\/balance/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });
    const balance = wallet?.balance?.get('BTCPC') || 0;

    const node = await Node.findOne({ account: user._id });
    const staked = node?.stake_amount || 0;

    const proofCount = await MiningProof.countDocuments({ miner: user.username });
    const dreamCount = await GenesisDream.countDocuments({ current_owner: user.username });

    bot.sendMessage(msg.chat.id, [
      `\u{1F4B0} *${esc(user.username)}* Balance`,
      ``,
      `Spendable: \`${fmt(balance)} BTCPC\``,
      `Staked: \`${fmt(staked)} BTCPC\``,
      `Total: \`${fmt(balance + staked)} BTCPC\``,
      ``,
      `Mining Proofs: ${proofCount} \\(soulbound\\)`,
      `Genesis Dreams: ${dreamCount} \\(NFTs\\)`,
    ].join('\n'), { parse_mode: 'MarkdownV2' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /mining ──
bot.onText(/\/mining/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const totalEpochs = await Epoch.countDocuments({ status: 'finalized' });
    const totalWork = await WorkProof.countDocuments({ node_id: user.username });
    const totalTokens = await WorkProof.aggregate([
      { $match: { node_id: user.username } },
      { $group: { _id: null, total: { $sum: '$tokens_generated' } } },
    ]);
    const tokens = totalTokens[0]?.total || 0;

    const recentEpochs = await Epoch.find({ status: 'finalized' })
      .sort({ epoch_number: -1 })
      .limit(5)
      .lean();

    const recentLines = recentEpochs.map(e => {
      const reward = e.rewards_distributed?.[0]?.amount || e.block_reward || 0;
      return `  Epoch ${e.epoch_number}: \\+${esc(fmt(reward))} BTCPC`;
    });

    bot.sendMessage(msg.chat.id, [
      `\u{26CF} *${esc(user.username)}* Mining Stats`,
      ``,
      `Epochs mined: ${totalEpochs}`,
      `Work proofs: ${totalWork}`,
      `Inference tokens generated: ${esc(tokens.toLocaleString())}`,
      ``,
      `*Recent epochs:*`,
      ...recentLines,
    ].join('\n'), { parse_mode: 'MarkdownV2' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /epoch ──
bot.onText(/\/epoch/, async (msg) => {
  try {
    const current = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
    if (!current) return bot.sendMessage(msg.chat.id, 'No epochs yet.');

    const reward = getBlockReward(current.epoch_number);

    bot.sendMessage(msg.chat.id, [
      `\u{1F4E6} *Epoch ${current.epoch_number}*`,
      ``,
      `Status: ${current.status}`,
      `Block reward: \`${fmt(reward, 8)} BTCPC\``,
      `Total work: ${current.total_work}`,
      `Difficulty: ${current.difficulty}`,
      `Started: ${current.started_at?.toISOString?.() || 'N/A'}`,
      current.ended_at ? `Ended: ${current.ended_at.toISOString()}` : `Still active`,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /network ──
bot.onText(/\/network/, async (msg) => {
  try {
    const totalEpochs = await Epoch.countDocuments();
    const finalizedEpochs = await Epoch.countDocuments({ status: 'finalized' });
    const activeNodes = await Node.countDocuments({ status: 'active' });
    const totalUsers = await User.countDocuments();
    const totalProofs = await WorkProof.countDocuments();

    const totalMined = await Epoch.aggregate([
      { $match: { status: 'finalized' } },
      { $group: { _id: null, total: { $sum: '$block_reward' } } },
    ]);
    const mined = totalMined[0]?.total || 0;

    const currentReward = getBlockReward(totalEpochs);

    bot.sendMessage(msg.chat.id, [
      `\u{1F310} *BTCPC Network*`,
      ``,
      `Total supply: 42,000,000 BTCPC`,
      `Mined so far: \`${fmt(mined)} BTCPC\``,
      `Current block reward: \`${fmt(currentReward, 8)} BTCPC\``,
      ``,
      `Epochs: ${finalizedEpochs} finalized`,
      `Active nodes: ${activeNodes}`,
      `Accounts: ${totalUsers}`,
      `Work proofs: ${totalProofs}`,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /proofs ──
bot.onText(/\/proofs/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const proofs = await MiningProof.find({ miner: user.username })
      .sort({ block_number: -1 })
      .limit(10)
      .lean();

    if (!proofs.length) return bot.sendMessage(msg.chat.id, 'No mining proofs yet.');

    const lines = proofs.map(p =>
      `  Block ${p.block_number}: ${fmt(p.reward_earned)} BTCPC | ${p.tokens_computed} tokens | ${p.model}`
    );

    bot.sendMessage(msg.chat.id, [
      `\u{1F3C5} *${esc(user.username)}* Mining Proofs \\(soulbound\\)`,
      ``,
      `Last ${proofs.length}:`,
      ...lines.map(l => esc(l)),
    ].join('\n'), { parse_mode: 'MarkdownV2' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /dreams ──
bot.onText(/\/dreams/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const dreams = await GenesisDream.find({ current_owner: user.username })
      .sort({ block_number: -1 })
      .limit(10)
      .lean();

    if (!dreams.length) return bot.sendMessage(msg.chat.id, 'No genesis dreams yet.');

    const lines = dreams.map(d => {
      const inscribed = d.inscription?.project ? `"${d.inscription.project}"` : '[uninscribed]';
      return `  Dream #${d.block_number} — ${inscribed} (miner: ${d.original_miner})`;
    });

    bot.sendMessage(msg.chat.id, [
      `\u{1F4AD} *Genesis Dreams* \\(${dreams.length} owned\\)`,
      ``,
      ...lines.map(l => esc(l)),
    ].join('\n'), { parse_mode: 'MarkdownV2' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /node ──
bot.onText(/\/node/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const node = await Node.findOne({ account: user._id }).lean();
    if (!node) return bot.sendMessage(msg.chat.id, 'No mining node registered for your account.');

    bot.sendMessage(msg.chat.id, [
      `\u{1F5A5} *Node Status*`,
      ``,
      `Status: ${node.status}`,
      `Models: ${node.models.join(', ') || 'none'}`,
      `Endpoint: \`${node.endpoint}\``,
      `Stake: ${fmt(node.stake_amount)} BTCPC`,
      `Reputation: ${node.reputation}/100`,
      `Last epoch: ${node.last_epoch_commitment}`,
      ``,
      `*Hardware:*`,
      `  GPU: ${node.hardware?.gpu || 'N/A'}`,
      `  VRAM: ${node.hardware?.vram_gb || 0} GB`,
      `  CPU: ${node.hardware?.cpu_cores || 0} cores`,
      `  RAM: ${node.hardware?.ram_gb || 0} GB`,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /history ──
bot.onText(/\/history/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const txs = await Transaction.find({
      $or: [{ from: user.username }, { to: user.username }],
    }).sort({ timestamp: -1 }).limit(10).lean();

    if (!txs.length) return bot.sendMessage(msg.chat.id, 'No transactions yet.');

    const lines = txs.map(tx => {
      const dir = tx.to === user.username ? '\u{2B06}' : '\u{2B07}';
      const other = tx.to === user.username ? tx.from : tx.to;
      return `  ${dir} ${tx.type}: ${fmt(tx.amount)} BTCPC ${tx.to === user.username ? 'from' : 'to'} ${other}`;
    });

    bot.sendMessage(msg.chat.id, [
      `\u{1F4DC} *Recent Transactions*`,
      ``,
      ...lines,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /reward ──
bot.onText(/\/reward/, async (msg) => {
  try {
    const latestEpoch = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
    const epochNum = latestEpoch ? latestEpoch.epoch_number : 0;
    const reward = getBlockReward(epochNum);
    const nextReward = getBlockReward(epochNum + 1);

    bot.sendMessage(msg.chat.id, [
      `\u{1F4B8} *Block Reward*`,
      ``,
      `Current epoch: ${epochNum}`,
      `Reward: \`${fmt(reward, 8)} BTCPC\``,
      `Next epoch reward: \`${fmt(nextReward, 8)} BTCPC\``,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /register <address> — register node for peer discovery ──
bot.onText(/\/register\s+(wss?:\/\/\S+)/, async (msg, match) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  const address = match[1];

  try {
    const node = await Node.findOne({ account: user._id }).lean();
    const gpu = node?.hardware?.gpu || null;

    await PeerRegistry.findOneAndUpdate(
      { username: user.username },
      {
        node_id: node?._id?.toString() || user._id.toString(),
        address,
        username: user.username,
        gpu,
        last_seen: new Date(),
      },
      { upsert: true, new: true }
    );

    const count = await PeerRegistry.countDocuments();
    bot.sendMessage(msg.chat.id, `\u{2705} Registered \`${address}\`\n${count} peer(s) in registry.`, { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /peers — list all registered peers ──
bot.onText(/\/peers/, async (msg) => {
  try {
    const peers = await PeerRegistry.find().sort({ last_seen: -1 }).limit(20).lean();
    if (!peers.length) return bot.sendMessage(msg.chat.id, 'No peers registered yet. Miners can register with `/register ws://ip:port`', { parse_mode: 'Markdown' });

    const lines = peers.map(p => {
      const ago = Math.round((Date.now() - new Date(p.last_seen).getTime()) / 60000);
      const gpu = p.gpu ? ` (${p.gpu})` : '';
      return `  \`${p.address}\` — ${p.username}${gpu} — ${ago}m ago`;
    });

    bot.sendMessage(msg.chat.id, [
      `\u{1F310} *Peer Registry* (${peers.length})`,
      ``,
      ...lines,
      ``,
      `Add to your .env:`,
      `\`BTCPC_SEED_PEERS=${peers.map(p => p.address).join(',')}\``,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /heartbeat — miner calls this periodically to stay in registry ──
bot.onText(/\/heartbeat/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return;

  try {
    const result = await PeerRegistry.findOneAndUpdate(
      { username: user.username },
      { last_seen: new Date() },
      { new: true }
    );
    if (result) {
      bot.sendMessage(msg.chat.id, `\u{1F493} Heartbeat recorded.`);
    } else {
      bot.sendMessage(msg.chat.id, 'Not registered. Use `/register ws://ip:port` first.', { parse_mode: 'Markdown' });
    }
  } catch (_) {}
});

// ── Error handling ──
bot.on('polling_error', (err) => {
  console.error('Polling error:', err.code || err.message);
});

process.on('uncaughtException', (err) => {
  console.error('Uncaught:', err.message, err.stack);
});
process.on('unhandledRejection', (reason) => {
  console.error('Unhandled:', reason);
});

// ── HTTP API for peer discovery (nodes call this on startup) ──
const express = require('express');
const httpApp = express();
httpApp.use(express.json());

// GET /peers — returns list of registered peer addresses
httpApp.get('/peers', async (_req, res) => {
  try {
    const peers = await PeerRegistry.find().sort({ last_seen: -1 }).limit(50).lean();
    res.json({ peers: peers.map(p => ({ address: p.address, username: p.username, gpu: p.gpu, last_seen: p.last_seen })) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /peers/register — node registers itself
httpApp.post('/peers/register', async (req, res) => {
  const { address, username, gpu } = req.body;
  if (!address || !username) return res.status(400).json({ error: 'address and username required' });

  try {
    await PeerRegistry.findOneAndUpdate(
      { username },
      { address, username, gpu, last_seen: new Date(), node_id: username },
      { upsert: true }
    );
    const count = await PeerRegistry.countDocuments();
    res.json({ ok: true, peers: count });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// POST /peers/heartbeat — keep-alive
httpApp.post('/peers/heartbeat', async (req, res) => {
  const { username } = req.body;
  if (!username) return res.status(400).json({ error: 'username required' });
  await PeerRegistry.findOneAndUpdate({ username }, { last_seen: new Date() });
  res.json({ ok: true });
});

httpApp.get('/health', (_req, res) => res.json({ status: 'ok', service: 'btcpc-bot' }));

const HTTP_PORT = process.env.BOT_HTTP_PORT || 3003;

// ── Connect and start ──
async function main() {
  console.log('[btcpc-bot] Connecting to MongoDB...');
  await mongoose.connect(MONGODB_URI);
  console.log('[btcpc-bot] MongoDB connected');
  httpApp.listen(HTTP_PORT, () => console.log(`[btcpc-bot] HTTP peer registry on port ${HTTP_PORT}`));
  console.log('[btcpc-bot] @btcpcbot is live');
}

main().catch(err => {
  console.error('[btcpc-bot] Fatal:', err.message);
  process.exit(1);
});
