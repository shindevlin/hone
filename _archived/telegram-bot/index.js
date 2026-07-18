require('dotenv').config({ path: require('path').resolve(__dirname, '.env') });
const TelegramBot = require('node-telegram-bot-api');
const mongoose = require('mongoose');

const BOT_TOKEN = process.env.HONE_BOT_TOKEN;
const MONGODB_URI = process.env.MONGODB_URI || 'mongodb://root:example@localhost:27017/hone?authSource=admin';

if (!BOT_TOKEN) {
  console.error('HONE_BOT_TOKEN required');
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
const WebSocket = require('ws');
const crypto = require('crypto');

const bot = new TelegramBot(BOT_TOKEN, {
  polling: { interval: 2000, autoStart: true, params: { timeout: 10 } }
});

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
    `\u{26D3} *HONE Node Bot*`,
    ``,
    `Link your account: \`/link <username>\``,
    `Then verify: \`/verify <challenge>\``,
    ``,
    `*Commands:*`,
    `/claim — get 1 free HONE (one-time)`,
    `/balance — HONE balance & staked`,
    `/mining — mining stats & recent epochs`,
    `/epoch — current epoch info`,
    `/network — network overview`,
    `/proofs — your mining proofs (soulbound)`,
    `/dreams — your genesis dreams (NFTs)`,
    `/node — your node status`,
    `/history — recent transactions`,
    `/reward — current block reward`,
    `/price [model] — inference pricing (model-aware)`,
    `/models — models available on the network`,
    `Just type anything to submit inference (0.001 HONE/token)`,
    `/peers — list registered P2P peers`,
    `/register <ws://ip:port> — register your node for peer discovery`,
    `/unlink — unlink Telegram account`,
  ].join('\n'), { parse_mode: 'Markdown' });
});

// ── /link <username> — start on-chain verification ──
const { startLink, verifySignedChallenge } = require('../src/services/telegramVerify');

bot.onText(/\/link\s+(\S+)/, async (msg, match) => {
  const chatId = msg.chat.id;
  const tgId = String(msg.from.id);
  const username = match[1].toLowerCase();

  try {
    const existing = await User.findOne({ telegramId: tgId });
    if (existing) {
      return bot.sendMessage(chatId, `Already linked to *${existing.username}*. Use /unlink first.`, { parse_mode: 'Markdown' });
    }

    const result = await startLink(username, tgId, msg.from.username);
    bot.sendMessage(chatId, [
      `*Verify ownership of ${username}*`,
      ``,
      `Sign this challenge with your posting key:`,
      `\`${result.challenge}\``,
      ``,
      `*How to sign (CLI):*`,
      `\`node bin/hone-cli sign-challenge ${result.challenge}\``,
      ``,
      `Then reply here with:`,
      `\`/verify <signature>:<recovery>\``,
      ``,
      `_Expires in ${result.expiresIn / 60} minutes_`,
    ].join('\n'), { parse_mode: 'Markdown' });

  } catch (err) {
    bot.sendMessage(chatId, `Error: ${err.message}`);
  }
});

// ── /verify <signature:recovery> — verify posting key signature ──
bot.onText(/\/verify\s+(\S+)/, async (msg, match) => {
  const chatId = msg.chat.id;
  const tgId = String(msg.from.id);
  const input = match[1];

  try {
    // Parse signature:recovery format
    const parts = input.split(':');
    if (parts.length !== 2) {
      return bot.sendMessage(chatId, 'Format: `/verify <signature>:<recovery>`\nGet this from: `node bin/hone-cli sign-challenge <challenge>`', { parse_mode: 'Markdown' });
    }

    const signature = parts[0];
    const recovery = parseInt(parts[1], 10);

    if (!/^[0-9a-f]{128}$/i.test(signature) || isNaN(recovery)) {
      return bot.sendMessage(chatId, 'Invalid signature format. Use the output from `hone-cli sign-challenge`.', { parse_mode: 'Markdown' });
    }

    // Find user with pending link for this telegram ID
    const user = await User.findOne({ 'pendingTelegramLink.telegramId': tgId });
    if (!user) {
      return bot.sendMessage(chatId, 'No pending link. Use `/link <username>` first.', { parse_mode: 'Markdown' });
    }

    // Verify signature against posting key
    const result = await verifySignedChallenge(user.username, tgId, signature, recovery);
    bot.sendMessage(chatId, `*${result.username}* verified and linked! Posting key signature verified on-chain (tx: \`${result.tx_id}\`).`, { parse_mode: 'Markdown' });

  } catch (err) {
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

// ── /claim — one-time faucet for new accounts ──
bot.onText(/\/claim/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const FAUCET_AMOUNT = 1;

    let wallet = await Wallet.findOne({ userId: user._id, chain: 'hone' });
    if (!wallet) {
      wallet = new Wallet({
        userId: user._id,
        chain: 'hone',
        address: 'hone_' + require('crypto').randomBytes(20).toString('hex'),
        balance: new Map([['HONE', 0]])
      });
    }

    const alreadyClaimed = await Transaction.findOne({ to: wallet.address, type: 'faucet' });
    if (alreadyClaimed) {
      return bot.sendMessage(msg.chat.id, `You've already claimed your starter tokens. To request more, email shindevlin@proton.me with your username.`);
    }

    const balance = wallet.balance.get('HONE') || 0;
    wallet.balance.set('HONE', balance + FAUCET_AMOUNT);
    await wallet.save();

    const tx = new Transaction({
      from: 'hone_faucet',
      to: wallet.address,
      amount: FAUCET_AMOUNT,
      type: 'faucet',
      memo: 'Welcome to HONE — starter tokens'
    });
    await tx.save();

    bot.sendMessage(msg.chat.id, [
      `\u{2705} *Claimed ${FAUCET_AMOUNT} HONE*`,
      ``,
      `Balance: \`${fmt(balance + FAUCET_AMOUNT)} HONE\``,
      ``,
      `You can now use inference \\(just type a message\\)`,
      `Need more? Email shin@hone\\.network`,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /balance ──
bot.onText(/\/balance/, async (msg) => {
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(msg.chat.id, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  try {
    const wallet = await Wallet.findOne({ userId: user._id, chain: 'hone' });
    const balance = wallet?.balance?.get('HONE') || 0;

    const node = await Node.findOne({ account: user._id });
    const staked = node?.stake_amount || 0;

    const proofCount = await MiningProof.countDocuments({ miner: user.username });
    const dreamCount = await GenesisDream.countDocuments({ current_owner: user.username });

    bot.sendMessage(msg.chat.id, [
      `\u{1F4B0} *${user.username}* Balance`,
      ``,
      `Spendable: \`${fmt(balance)} HONE\``,
      `Staked: \`${fmt(staked)} HONE\``,
      `Total: \`${fmt(balance + staked)} HONE\``,
      ``,
      `Mining Proofs: ${proofCount} \\(soulbound\\)`,
      `Genesis Dreams: ${dreamCount} \\(NFTs\\)`,
    ].join('\n'), { parse_mode: 'Markdown' });
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
      return `  Epoch ${e.epoch_number}: \\+${esc(fmt(reward))} HONE`;
    });

    bot.sendMessage(msg.chat.id, [
      `\u{26CF} *${user.username}* Mining Stats`,
      ``,
      `Epochs mined: ${totalEpochs}`,
      `Work proofs: ${totalWork}`,
      `Inference tokens generated: ${esc(tokens.toLocaleString())}`,
      ``,
      `*Recent epochs:*`,
      ...recentLines,
    ].join('\n'), { parse_mode: 'Markdown' });
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
      `Block reward: \`${fmt(reward, 8)} HONE\``,
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
      `\u{1F310} *HONE Network*`,
      ``,
      `Total supply: 42,000,000 HONE`,
      `Mined so far: \`${fmt(mined)} HONE\``,
      `Current block reward: \`${fmt(currentReward, 8)} HONE\``,
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
      `  Block ${p.block_number}: ${fmt(p.reward_earned)} HONE | ${p.tokens_computed} tokens | ${p.model}`
    );

    bot.sendMessage(msg.chat.id, [
      `\u{1F3C5} *${user.username}* Mining Proofs \\(soulbound\\)`,
      ``,
      `Last ${proofs.length}:`,
      ...lines.map(l => esc(l)),
    ].join('\n'), { parse_mode: 'Markdown' });
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
    ].join('\n'), { parse_mode: 'Markdown' });
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
      `Stake: ${fmt(node.stake_amount)} HONE`,
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
      return `  ${dir} ${tx.type}: ${fmt(tx.amount)} HONE ${tx.to === user.username ? 'from' : 'to'} ${other}`;
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
      `Reward: \`${fmt(reward, 8)} HONE\``,
      `Next epoch reward: \`${fmt(nextReward, 8)} HONE\``,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /price [model] — check current inference pricing ──
bot.onText(/\/price\s*(.*)/, async (msg, match) => {
  try {
    const { getCurrentPricing } = require('../src/services/pricing');
    const model = match[1]?.trim() || undefined;
    const p = await getCurrentPricing(model);
    bot.sendMessage(msg.chat.id, [
      `\u{1F4B0} *HONE Inference Pricing*`,
      model ? `Model: \`${model}\`` : '',
      ``,
      `1 HONE = ${p.tokensPerHone} tokens`,
      `Cost per token: ${p.costPerToken.toFixed(6)} HONE`,
      `Network load: ${(p.load * 100).toFixed(1)}%`,
      `Load multiplier: ${p.loadMultiplier}x`,
      `Model weight: ${p.modelWeight}x`,
      `Total multiplier: ${p.totalMultiplier}x`,
      ``,
      `_Bigger models cost more. Busier network costs more._`,
      `_Try: /price dolphin-llama3:8b_`,
    ].join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── /models — list models available on the network ──
bot.onText(/\/models/, async (msg) => {
  try {
    const { getNetworkModels, getUnmetDemand } = require('../src/services/modelRegistry');
    const models = await getNetworkModels();
    const demand = getUnmetDemand();

    const lines = models.length > 0
      ? models.slice(0, 15).map(m => `  \`${m.model}\` — ${m.miners} miner(s), avg rep ${m.avg_reputation}`)
      : ['  No models registered yet'];

    const demandLines = demand.length > 0
      ? demand.slice(0, 5).map(d => `  \`${d.model}\` — ${d.requests} request(s) waiting`)
      : [];

    const parts = [
      `\u{1F5A5} *Network Models*`,
      ``,
      `*Available:*`,
      ...lines,
    ];

    if (demandLines.length > 0) {
      parts.push('', '*Wanted (no miner has these yet):*', ...demandLines,
        '', '_Miners: pull these models to earn from unmet demand_');
    }

    bot.sendMessage(msg.chat.id, parts.join('\n'), { parse_mode: 'Markdown' });
  } catch (err) {
    bot.sendMessage(msg.chat.id, `Error: ${err.message}`);
  }
});

// ── Inference via relay ──
const RELAY_URL = process.env.HONE_RELAY_URL || 'https://hone-relay.shindevlin.workers.dev';
const RELAY_API_KEY = process.env.HONE_RELAY_API_KEY || 'hone_0236fb3a9c63dc7e556bfeed5dc92290';
const axios = require('axios');

// ── Default message handler — any text that isn't a command goes to inference ──
bot.on('message', async (msg) => {
  if (!msg.text || msg.text.startsWith('/')) return; // skip commands
  const chatId = msg.chat.id;
  const prompt = msg.text.trim();

  if (!prompt) return bot.sendMessage(chatId, 'Usage: `/ask your question here`', { parse_mode: 'Markdown' });

  // Check user has linked account and sufficient balance
  const user = await getLinkedUser(msg.from.id);
  if (!user) return bot.sendMessage(chatId, 'Link your account first: `/link <username>`', { parse_mode: 'Markdown' });

  const Wallet = require('../src/models/Wallet');
  const wallet = await Wallet.findOne({ userId: user._id, chain: 'hone' });
  const balance = wallet?.balance?.get('HONE') || 0;
  // Check user has staked at least 1 HONE for bot access
  const StakingPool = require('../src/models/StakingPool');
  const stake = await StakingPool.findOne({ account: user._id, status: 'active' });
  const staked = stake?.staked_amount || 0;
  if (staked < 1) {
    return bot.sendMessage(chatId, `\u{274C} You need at least 1 HONE staked to use inference. Currently staked: ${fmt(staked)}. Use the CLI to stake.`);
  }

  const minBalance = 0.01; // minimum to submit
  if (balance < minBalance) {
    return bot.sendMessage(chatId, `\u{274C} Insufficient balance. You have ${fmt(balance)} HONE, need at least ${minBalance}.`);
  }

  const statusMsg = await bot.sendMessage(chatId, '\u{1F504} Submitting to HONE network...');

  try {
    // Submit async job via local API
    const API_URL = process.env.HONE_API_URL || 'http://localhost:3000';
    const submitRes = await axios.post(`${API_URL}/v1/inference/submit`, {
      model: 'qwen3.5:27b',
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 1024
    }, {
      timeout: 10000,
      headers: { 'Authorization': `Bearer ${RELAY_API_KEY}` },
    });

    const jobId = submitRes.data.job_id;

    // Update status message
    bot.editMessageText(`\u{2699} Job submitted. Waiting for miner...`, {
      chat_id: chatId, message_id: statusMsg.message_id
    }).catch(() => {});

    // Poll for result
    const InferenceJob = require('../src/models/InferenceJob');
    const MAX_WAIT = 300000; // 5 min
    const POLL_INTERVAL = 3000; // 3s
    const startTime = Date.now();
    let job = null;

    while (Date.now() - startTime < MAX_WAIT) {
      job = await InferenceJob.findOne({ job_id: jobId }).lean();
      if (job && (job.status === 'completed' || job.status === 'failed')) break;

      // Update status periodically
      const elapsed = Math.round((Date.now() - startTime) / 1000);
      if (elapsed % 15 === 0 && elapsed > 0) {
        bot.editMessageText(`\u{2699} Processing... (${elapsed}s, status: ${job?.status || 'pending'})`, {
          chat_id: chatId, message_id: statusMsg.message_id
        }).catch(() => {});
      }

      await new Promise(r => setTimeout(r, POLL_INTERVAL));
    }

    if (!job || job.status !== 'completed') {
      bot.editMessageText(`\u{23F3} Job still processing. Check back with /status ${jobId}`, {
        chat_id: chatId, message_id: statusMsg.message_id
      }).catch(() => {});
      return;
    }

    const text = job.result_text || 'No response';
    const tokens = job.tokens_generated || 0;
    const elapsed = job.elapsed_ms || 0;
    const node = job.node_name || 'unknown';

    // Dynamic pricing
    const { calculateCost } = require('../src/services/pricing');
    const { cost: inferCost } = await calculateCost(tokens, 'qwen3.5:27b');
    const cost = Math.max(0.001, inferCost);
    const newBalance = balance - cost;

    // Deduct
    wallet.balance.set('HONE', Math.max(0, newBalance));
    await wallet.save();

    // Delete status message
    bot.deleteMessage(chatId, statusMsg.message_id).catch(() => {});

    // Send result
    const maxLen = 3600;
    const truncated = text.length > maxLen ? text.slice(0, maxLen) + '\n\n... (truncated)' : text;
    const footer = `\n\n\u{26D3} ${tokens} tokens | ${fmt(cost)} HONE | bal: ${fmt(newBalance)} | ${(elapsed / 1000).toFixed(1)}s | ${node}`;

    bot.sendMessage(chatId, truncated + footer).catch(() => {
      bot.sendMessage(chatId, text.slice(0, 3600) + footer);
    });
  } catch (err) {
    // No deduction happened yet (we deduct after success), so no refund needed
    bot.deleteMessage(chatId, status.message_id).catch(() => {});
    const errMsg = err.response?.data?.error || err.message;
    bot.sendMessage(chatId, `\u{274C} Inference failed: ${errMsg}`);
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
      `\`HONE_SEED_PEERS=${peers.map(p => p.address).join(',')}\``,
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

httpApp.get('/health', (_req, res) => res.json({ status: 'ok', service: 'hone-bot' }));

const HTTP_PORT = process.env.BOT_HTTP_PORT || 3003;

// ── Connect and start ──
async function main() {
  console.log('[hone-bot] Connecting to MongoDB...');
  await mongoose.connect(MONGODB_URI);
  console.log('[hone-bot] MongoDB connected');
  httpApp.listen(HTTP_PORT, () => console.log(`[hone-bot] HTTP peer registry on port ${HTTP_PORT}`));
  console.log('[hone-bot] @honebot is live');
}

main().catch(err => {
  console.error('[hone-bot] Fatal:', err.message);
  process.exit(1);
});
