require('dotenv').config();
const TelegramBot = require('node-telegram-bot-api');
const axios = require('axios');

const BOT_TOKEN = process.env.HONE_BOT_TOKEN;
const API_URL   = process.env.HONE_API_URL || 'https://hone.net';
const BOT_KEY   = process.env.BOT_API_KEY;

if (!BOT_TOKEN) { console.error('HONE_BOT_TOKEN required'); process.exit(1); }
if (!BOT_KEY)   { console.error('BOT_API_KEY required'); process.exit(1); }

const bot = new TelegramBot(BOT_TOKEN, {
  polling: { interval: 2000, autoStart: true, params: { timeout: 10 } }
});

// ── Conversation state (per chat) ────────────────────────────────────────────
// step values:
//   IDLE | WELCOME
//   LOGIN_USER | LOGIN_PASS
//   IMPORT_PHRASE
//   CREATE_USER | CREATE_PASS | CREATE_PASS_CONFIRM
//   QUIZ_1 | QUIZ_2
//   DELETE_CHOICE | DELETE_WARN
//   MAIN_MENU
//   SEND_TO | SEND_AMOUNT
//   SETTINGS
const chatState = new Map();

function getState(chatId) {
  return chatState.get(chatId) || { step: 'IDLE', data: {} };
}

function setState(chatId, step, data = {}) {
  const current = getState(chatId);
  chatState.set(chatId, { step, data: { ...current.data, ...data } });
}

function clearState(chatId) {
  chatState.set(chatId, { step: 'IDLE', data: {} });
}

// ── API helper with retry ─────────────────────────────────────────────────────
const apiClient = axios.create({
  baseURL: `${API_URL}/api/bot`,
  headers: { 'x-bot-key': BOT_KEY, 'Content-Type': 'application/json' },
  timeout: 15000
});

async function apiCall(method, path, body, retries = 3) {
  let lastErr;
  for (let attempt = 0; attempt < retries; attempt++) {
    try {
      if (attempt > 0) await new Promise(r => setTimeout(r, 3000));
      const res = method === 'GET'
        ? await apiClient.get(path, { params: body })
        : await apiClient.post(path, body);
      return res.data;
    } catch (err) {
      lastErr = err;
    }
  }
  throw lastErr;
}

// ── Inline keyboard builder ───────────────────────────────────────────────────
// rows: array of rows; each row is array of strings or { text, callback_data } objects
function kb(rows) {
  return {
    reply_markup: {
      inline_keyboard: rows.map(row =>
        row.map(b => typeof b === 'string'
          ? { text: b, callback_data: b }
          : b
        )
      )
    }
  };
}

function fmt(n, d = 4) {
  return Number(n || 0).toFixed(d);
}

// ── Utility: safe delete ──────────────────────────────────────────────────────
async function safeDelete(chatId, msgId) {
  if (!msgId) return;
  try { await bot.deleteMessage(chatId, msgId); } catch (_) {}
}

// ── Utility: send with retry prompt on total failure ─────────────────────────
async function withRetryMsg(chatId, apiCallFn) {
  try {
    return await apiCallFn();
  } catch (err) {
    const apiErr = err.response?.data?.error;
    if (apiErr) {
      // Business logic error — surface it cleanly
      await bot.sendMessage(chatId, `❌ ${apiErr}`, kb([
        [{ text: '← Back to Menu', callback_data: 'MENU' }]
      ]));
    } else {
      await bot.sendMessage(chatId, 'Having trouble connecting. Try again in a minute.', kb([
        [{ text: 'Try Again', callback_data: 'RETRY_MENU' }]
      ]));
    }
    return null;
  }
}

// ── Flows ─────────────────────────────────────────────────────────────────────

async function showWelcome(chatId) {
  clearState(chatId);
  setState(chatId, 'WELCOME');
  await bot.sendMessage(chatId,
    '🟠 *Welcome to HONE!*\nBitcoin Proof of Compute — every token earned.\n\nDo you already have a HONE wallet?',
    { parse_mode: 'Markdown', ...kb([
      [
        { text: 'Yes, I have one', callback_data: 'HAS_WALLET' },
        { text: 'No, create one', callback_data: 'CREATE_NEW' }
      ]
    ])}
  );
}

async function showHasWallet(chatId) {
  setState(chatId, 'HAS_WALLET');
  await bot.sendMessage(chatId,
    'How would you like to sign in?',
    kb([
      [
        { text: 'Username + Password', callback_data: 'LOGIN_USER_PASS' },
        { text: 'Import 12-word phrase', callback_data: 'IMPORT_PHRASE' }
      ],
      [{ text: '← Back', callback_data: 'BACK_WELCOME' }]
    ])
  );
}

async function startLoginUserPass(chatId) {
  setState(chatId, 'LOGIN_USER');
  await bot.sendMessage(chatId, 'Enter your HONE username:');
}

async function startImportPhrase(chatId) {
  setState(chatId, 'IMPORT_PHRASE');
  await bot.sendMessage(chatId,
    'Enter your 12-word backup phrase:\n\n_Your message will be deleted immediately after processing._',
    { parse_mode: 'Markdown' }
  );
}

async function startCreateNew(chatId) {
  setState(chatId, 'CREATE_USER');
  await bot.sendMessage(chatId,
    'Choose a username (3–20 chars, lowercase letters, numbers, hyphens):',
    kb([[{ text: '← Back', callback_data: 'BACK_WELCOME' }]])
  );
}

async function showMainMenu(chatId, username, balance) {
  setState(chatId, 'MAIN_MENU', { username, balance });
  const balStr = balance !== undefined ? `\nBalance: *${fmt(balance)} HONE*` : '';
  await bot.sendMessage(chatId,
    `🟠 *HONE — ${username}*${balStr}`,
    { parse_mode: 'Markdown', ...kb([
      [
        { text: '💰 Balance', callback_data: 'ACT_BALANCE' },
        { text: '💸 Send',    callback_data: 'ACT_SEND' },
        { text: '📥 Receive', callback_data: 'ACT_RECEIVE' }
      ],
      [
        { text: '💱 Bridge',   callback_data: 'ACT_BRIDGE' },
        { text: '⚙️ Settings', callback_data: 'ACT_SETTINGS' },
        { text: '📡 Earn',     callback_data: 'ACT_EARN' }
      ]
    ])}
  );
}

async function showAddressBook(chatId, wallets) {
  const lines = [
    `*Your wallets on 6 chains:*\n`,
    `🔶 HONE:    \`${wallets.hone || wallets.username || '—'}\``,
    `🔷 Ethereum:  \`${wallets.evm || '—'}\``,
    `🟣 Solana:    \`${wallets.solana || '—'}\``,
    `🟠 Bitcoin:   \`${wallets.bitcoin || '—'}\``,
    `💎 TON:       \`${wallets.ton || '—'}\``,
    `🟢 Hive:      \`${wallets.hive || wallets.username || '—'}\``,
  ];
  if (wallets.publicKey) {
    lines.push(`\n🔑 *Public key:* \`${wallets.publicKey}\``);
  }
  lines.push('\nAnyone can send you tokens on ANY of these chains.\nYou can receive without installing any software.');

  await bot.sendMessage(chatId, lines.join('\n'), { parse_mode: 'Markdown' });
}

async function showEarnOptions(chatId) {
  await bot.sendMessage(chatId,
    'How would you like to earn HONE?',
    kb([
      [{ text: '⏰ Clock (any device)', callback_data: 'EARN_CLOCK' }],
      [{ text: '🖥️ Mine (GPU)',         callback_data: 'EARN_MINE'  }],
      [{ text: '💾 Host Storage',       callback_data: 'EARN_STORE' }],
      [{ text: '📡 IoT Sensor',         callback_data: 'EARN_IOT'   }],
      [{ text: '← Back',                callback_data: 'MENU'       }]
    ])
  );
}

async function showSpendOptions(chatId) {
  await bot.sendMessage(chatId,
    'What would you like to do?',
    kb([
      [
        { text: '💸 Send',     callback_data: 'ACT_SEND'    },
        { text: '📥 Receive',  callback_data: 'ACT_RECEIVE' }
      ],
      [
        { text: '💰 Balance',  callback_data: 'ACT_BALANCE' },
        { text: '💱 Bridge',   callback_data: 'ACT_BRIDGE'  }
      ],
      [{ text: '← Back', callback_data: 'MENU' }]
    ])
  );
}

async function showSettings(chatId) {
  setState(chatId, 'SETTINGS');
  await bot.sendMessage(chatId,
    '⚙️ *Settings*',
    { parse_mode: 'Markdown', ...kb([
      [{ text: '📋 Show Addresses', callback_data: 'SET_ADDRESSES' }],
      [{ text: '🔐 Export Backup',  callback_data: 'SET_EXPORT'    }],
      [{ text: '🔗 Link Account',   callback_data: 'SET_LINK'      }],
      [{ text: '💔 Heartbeat',      callback_data: 'SET_HEARTBEAT' }],
      [{ text: '← Back',            callback_data: 'MENU'          }]
    ])}
  );
}

// ── Mnemonic quiz helpers ─────────────────────────────────────────────────────

function pickQuizWords(mnemonic) {
  const words = mnemonic.trim().split(/\s+/);
  const used = new Set();
  function pick() {
    let i;
    do { i = Math.floor(Math.random() * 12); } while (used.has(i));
    used.add(i);
    return i;
  }
  const i1 = pick();
  const i2 = pick();
  return [
    { index: i1, word: words[i1] },
    { index: i2, word: words[i2] }
  ];
}

// ── Callback query handler ────────────────────────────────────────────────────

bot.on('callback_query', async (query) => {
  const chatId  = query.message.chat.id;
  const action  = query.data;
  const msgId   = query.message.message_id;
  const telegramId = String(query.from.id);

  await bot.answerCallbackQuery(query.id).catch(() => {});

  const s = getState(chatId);

  // ── Welcome screen buttons ──
  if (action === 'HAS_WALLET')       return showHasWallet(chatId);
  if (action === 'CREATE_NEW')       return startCreateNew(chatId);
  if (action === 'BACK_WELCOME')     return showWelcome(chatId);
  if (action === 'LOGIN_USER_PASS')  return startLoginUserPass(chatId);
  if (action === 'IMPORT_PHRASE')    return startImportPhrase(chatId);

  // ── Main menu navigation ──
  if (action === 'MENU' || action === 'RETRY_MENU') {
    const data = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    if (!data) return;
    return showMainMenu(chatId, data.username, data.balance);
  }

  // ── Main menu actions ──
  if (action === 'ACT_BALANCE') {
    const data = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    if (!data) return;
    await bot.sendMessage(chatId, [
      `*${data.username}* Balance`,
      ``,
      `Spendable: \`${fmt(data.balance)} HONE\``,
      `Staked:    \`${fmt(data.staked)} HONE\``,
      `Total:     \`${fmt(data.balance + data.staked)} HONE\``,
      `Proofs: ${data.proofCount || 0} | Dreams: ${data.dreamCount || 0}`,
    ].join('\n'), { parse_mode: 'Markdown', ...kb([[{ text: '← Menu', callback_data: 'MENU' }]]) });
    return;
  }

  if (action === 'ACT_RECEIVE') {
    const data = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    if (!data) return;
    await bot.sendMessage(chatId,
      `📥 *Your HONE address:*\n\`${data.username}\`\n\nShare your username to receive HONE.`,
      { parse_mode: 'Markdown', ...kb([[{ text: '← Menu', callback_data: 'MENU' }]]) }
    );
    return;
  }

  if (action === 'ACT_SEND') {
    setState(chatId, 'SEND_TO');
    await bot.sendMessage(chatId,
      '💸 *Send HONE*\n\nEnter the recipient username:',
      { parse_mode: 'Markdown', ...kb([[{ text: '✕ Cancel', callback_data: 'MENU' }]]) }
    );
    return;
  }

  if (action === 'ACT_BRIDGE') {
    await bot.sendMessage(chatId,
      '💱 *Bridge*\n\nCross-chain bridging coming soon.\nVisit hone.net for bridge partners.',
      { parse_mode: 'Markdown', ...kb([[{ text: '← Menu', callback_data: 'MENU' }]]) }
    );
    return;
  }

  if (action === 'ACT_EARN') return showEarnOptions(chatId);
  if (action === 'ACT_SPEND') return showSpendOptions(chatId);
  if (action === 'ACT_SETTINGS') return showSettings(chatId);

  // ── Earn sub-options ──
  const EARN_INFO = {
    EARN_CLOCK: {
      title: '⏰ Clock Node',
      body: 'Run a clock node on any device — laptop, Raspberry Pi, phone.\n\n`npm install -g hone-clock && hone-clock --account <username>`\n\nMore: hone.net/install/clock'
    },
    EARN_MINE: {
      title: '🖥️ GPU Mining',
      body: 'Mine HONE with any Ollama-compatible GPU.\n\n`npm install -g hone && hone-mine --miner <username>`\n\nRequires: Ollama + any model (qwen, llama, mistral, gemma, deepseek...)\nMore: hone.net/install/mine'
    },
    EARN_STORE: {
      title: '💾 Storage Node',
      body: 'Earn HONE by hosting files for the HONE-FS layer.\n\n`npm install -g hone-storage && hone-storage --account <username>`\n\nMore: hone.net/install/storage'
    },
    EARN_IOT: {
      title: '📡 IoT Sensor / Gateway',
      body: 'Connect sensors or run a gateway node to earn HONE for providing real-world data.\n\nMore: hone.net/install/iot'
    }
  };

  if (EARN_INFO[action]) {
    const info = EARN_INFO[action];
    await bot.sendMessage(chatId,
      `*${info.title}*\n\n${info.body}`,
      { parse_mode: 'Markdown', ...kb([[{ text: '← Earn', callback_data: 'ACT_EARN' }, { text: '⌂ Menu', callback_data: 'MENU' }]]) }
    );
    return;
  }

  // ── Settings sub-options ──
  if (action === 'SET_ADDRESSES') {
    const data = await withRetryMsg(chatId, () =>
      apiCall('GET', '/addresses', { telegramId })
    );
    if (!data) return;
    // Server nests chain addresses under data.addresses and keys under data.public_keys
    const wallets = {
      username: data.username,
      hone: data.addresses?.hone || data.username || '—',
      evm: data.addresses?.evm || null,
      solana: data.addresses?.solana || null,
      bitcoin: data.addresses?.bitcoin || null,
      ton: data.addresses?.ton || null,
      hive: data.addresses?.hive || data.username || null,
      publicKey: data.public_keys?.owner || null,
    };
    await showAddressBook(chatId, wallets);
    await bot.sendMessage(chatId, '​', kb([[{ text: '← Settings', callback_data: 'ACT_SETTINGS' }]]));
    return;
  }

  if (action === 'SET_EXPORT') {
    setState(chatId, 'EXPORT_CONFIRM');
    await bot.sendMessage(chatId,
      '🔐 *Export Backup Phrase*\n\n⚠️ Your backup phrase gives FULL access to your account. Make sure nobody can see your screen.\n\nContinue?',
      { parse_mode: 'Markdown', ...kb([
        [
          { text: 'Yes, show it', callback_data: 'EXPORT_DO' },
          { text: 'Cancel',       callback_data: 'ACT_SETTINGS' }
        ]
      ])}
    );
    return;
  }

  // TODO: POST /api/bot/export-mnemonic does not exist on the server yet — needs server-side implementation
  if (action === 'EXPORT_DO') {
    await bot.sendMessage(chatId,
      '⚠️ Mnemonic export is not available via Telegram yet. Use the CLI (`hone-cli export`) to export your backup phrase.',
      kb([[{ text: '← Settings', callback_data: 'ACT_SETTINGS' }]])
    );
    return;
  }

  if (action.startsWith('DELETE_EXPORT:')) {
    const targetId = parseInt(action.split(':')[1], 10);
    await safeDelete(chatId, targetId);
    await bot.sendMessage(chatId,
      '🗑️ Deleted from this chat. Keep your backup phrase in a safe offline location.',
      kb([[{ text: '← Settings', callback_data: 'ACT_SETTINGS' }]])
    );
    return;
  }

  if (action === 'SET_LINK') {
    setState(chatId, 'LINK_USER');
    await bot.sendMessage(chatId,
      '🔗 *Link Account*\n\nEnter the HONE username to link to this Telegram account:',
      { parse_mode: 'Markdown', ...kb([[{ text: '✕ Cancel', callback_data: 'ACT_SETTINGS' }]]) }
    );
    return;
  }

  if (action === 'SET_HEARTBEAT') {
    const data = await withRetryMsg(chatId, () =>
      apiCall('POST', '/heartbeat', { telegramId })
    );
    if (!data) return;
    await bot.sendMessage(chatId,
      '💔 *Heartbeat recorded*\n\nYour dormancy clock is reset. Tokens safe for another 5 years.\n\n_Any wallet activity resets it automatically — you only need this if you\'ve been inactive._',
      { parse_mode: 'Markdown', ...kb([[{ text: '← Settings', callback_data: 'ACT_SETTINGS' }]]) }
    );
    return;
  }

  // ── Mnemonic deletion choice ──
  if (action === 'MNEMONIC_DELETE_YES') {
    const mnemonicMsgId = s.data.mnemonicMsgId;
    await safeDelete(chatId, mnemonicMsgId);
    setState(chatId, 'MAIN_MENU');
    await bot.sendMessage(chatId, [
      '🗑️ Deleted.',
      '',
      'Your *password* works for all daily operations.',
      'The backup phrase is only needed if you lose both your password AND your Telegram access.',
      'If that happens, recovery is mathematically impossible — not because we won\'t help, but because the cryptography makes it impossible for anyone (including us) to reconstruct your keys.',
      '',
      'This is actually a *good* thing — it means nobody can steal your account either.',
      '',
      'You\'re in control. That\'s the point. 🔒'
    ].join('\n'), { parse_mode: 'Markdown' });
    const balData = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    if (balData) await showMainMenu(chatId, balData.username, balData.balance);
    return;
  }

  if (action === 'MNEMONIC_DELETE_NO') {
    await bot.sendMessage(chatId, [
      '⚠️ This phrase gives *FULL* access to your account.',
      'Telegram stores chat history on their servers.',
      'You\'re keeping it here at your own risk.',
    ].join('\n'), { parse_mode: 'Markdown', ...kb([
      [
        { text: 'I understand, keep it',   callback_data: 'MNEMONIC_KEEP_CONFIRM' },
        { text: 'Actually, delete it 🗑️', callback_data: 'MNEMONIC_DELETE_YES'   }
      ]
    ])} );
    return;
  }

  if (action === 'MNEMONIC_KEEP_CONFIRM') {
    setState(chatId, 'MAIN_MENU');
    const balData = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    if (balData) await showMainMenu(chatId, balData.username, balData.balance);
    return;
  }

  // ── Earn/Spend post-creation choice ──
  if (action === 'EARN_HONE') return showEarnOptions(chatId);
  if (action === 'USE_SPEND')  return showSpendOptions(chatId);

  // ── Ready for quiz ──
  if (action === 'QUIZ_READY') {
    const mnemonic = s.data.mnemonic;
    const [q1] = pickQuizWords(mnemonic);
    setState(chatId, 'QUIZ_1', { quizWord1: q1 });
    await bot.sendMessage(chatId,
      `What is word *#${q1.index + 1}* of your backup phrase?`,
      { parse_mode: 'Markdown' }
    );
    return;
  }
});

// ── Text message handler (typed input) ───────────────────────────────────────

bot.on('message', async (msg) => {
  if (!msg.text) return;
  const chatId = msg.chat.id;
  const text = msg.text.trim();
  const telegramId = String(msg.from.id);
  const msgId = msg.message_id;

  if (text.startsWith('/start')) {
    return showWelcome(chatId);
  }

  // Ignore other slash commands in the new flow
  if (text.startsWith('/')) return;

  const s = getState(chatId);

  // ── LOGIN: waiting for username ──
  if (s.step === 'LOGIN_USER') {
    setState(chatId, 'LOGIN_PASS', { loginUsername: text.toLowerCase() });
    await bot.sendMessage(chatId,
      `Enter your password for *${text.toLowerCase()}*:\n\n_Your message will be deleted immediately._`,
      { parse_mode: 'Markdown' }
    );
    return;
  }

  // ── LOGIN: waiting for password ──
  if (s.step === 'LOGIN_PASS') {
    await safeDelete(chatId, msgId);
    const { loginUsername } = s.data;
    const statusMsg = await bot.sendMessage(chatId, 'Signing in...');
    const data = await withRetryMsg(chatId, () =>
      apiCall('POST', '/login', { username: loginUsername, password: text, telegramId })
    );
    await safeDelete(chatId, statusMsg.message_id);
    if (!data) return;
    // Login response has no balance — fetch it separately
    const balData = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    const balance = balData ? balData.balance : 0;
    await bot.sendMessage(chatId,
      `✅ Welcome back, *${data.username}*!`,
      { parse_mode: 'Markdown' }
    );
    await showMainMenu(chatId, data.username, balance);
    return;
  }

  // ── IMPORT: waiting for mnemonic ──
  // TODO: POST /api/bot/import does not exist on the server yet — needs server-side implementation
  if (s.step === 'IMPORT_PHRASE') {
    await safeDelete(chatId, msgId);
    await bot.sendMessage(chatId,
      '⚠️ Wallet import via mnemonic is not available yet. Please sign in with username + password instead.',
      kb([[{ text: '← Back', callback_data: 'HAS_WALLET' }]])
    );
    return;
  }

  // ── CREATE: waiting for username ──
  if (s.step === 'CREATE_USER') {
    const username = text.toLowerCase();
    if (!/^[a-z0-9][a-z0-9._-]{1,18}[a-z0-9]$/.test(username) || username.length < 3) {
      await bot.sendMessage(chatId,
        '❌ Invalid username. Use 3–20 lowercase letters, numbers, hyphens, dots, or underscores.\n\nTry again:',
        kb([[{ text: '← Back', callback_data: 'BACK_WELCOME' }]])
      );
      return;
    }
    setState(chatId, 'CREATE_PASS', { newUsername: username });
    await bot.sendMessage(chatId,
      `Create a password for *${username}*.\n\n_You'll also get a 12-word backup phrase.\nYour message will be deleted immediately._`,
      { parse_mode: 'Markdown' }
    );
    return;
  }

  // ── CREATE: waiting for password ──
  if (s.step === 'CREATE_PASS') {
    await safeDelete(chatId, msgId);
    if (text.length < 8) {
      await bot.sendMessage(chatId, '❌ Password must be at least 8 characters. Try again:');
      return;
    }
    setState(chatId, 'CREATE_PASS_CONFIRM', { newPassword: text, newPasswordMsgId: msgId });
    await bot.sendMessage(chatId,
      'Confirm your password:\n\n_This message will also be deleted._',
      { parse_mode: 'Markdown' }
    );
    return;
  }

  // ── CREATE: confirm password ──
  if (s.step === 'CREATE_PASS_CONFIRM') {
    await safeDelete(chatId, msgId);
    if (text !== s.data.newPassword) {
      await bot.sendMessage(chatId,
        '❌ Passwords don\'t match. Let\'s start over.\n\nCreate a password:',
        { parse_mode: 'Markdown' }
      );
      setState(chatId, 'CREATE_PASS');
      return;
    }

    const { newUsername, newPassword } = s.data;
    const statusMsg = await bot.sendMessage(chatId, 'Creating account...');

    const data = await withRetryMsg(chatId, () =>
      apiCall('POST', '/create', { username: newUsername, password: newPassword, telegramId, telegramUsername: msg.from.username })
    );
    await safeDelete(chatId, statusMsg.message_id);
    if (!data) return;

    await bot.sendMessage(chatId,
      `✅ Account *${newUsername}* created!\n\nYour 12-word backup phrase:`,
      { parse_mode: 'Markdown' }
    );

    const phraseMsg = await bot.sendMessage(chatId, data.mnemonic || data.phrase || '(phrase unavailable)');

    setState(chatId, 'SAVE_GUIDANCE', { mnemonic: data.mnemonic || data.phrase, mnemonicMsgId: phraseMsg.message_id });

    await bot.sendMessage(chatId, [
      '📋 *Save your backup phrase somewhere safe:*',
      '',
      '• 📝 Write it on paper (safest)',
      '• 💬 Forward to Saved Messages',
      '• 📱 Save in Signal "Note to Self"',
      '',
      'When you\'re ready, I\'ll quiz you on two words to make sure you saved it.',
    ].join('\n'), { parse_mode: 'Markdown', ...kb([
      [{ text: "I'm ready for the quiz ✅", callback_data: 'QUIZ_READY' }]
    ])} );
    return;
  }

  // ── QUIZ word 1 answer ──
  if (s.step === 'QUIZ_1') {
    await safeDelete(chatId, msgId);
    const { quizWord1, mnemonic } = s.data;
    if (text.toLowerCase().trim() !== quizWord1.word.toLowerCase()) {
      await bot.sendMessage(chatId,
        `❌ That's not right. What is word *#${quizWord1.index + 1}*?`,
        { parse_mode: 'Markdown' }
      );
      return;
    }
    // Ask word 2
    const words = mnemonic.trim().split(/\s+/);
    let idx2;
    do { idx2 = Math.floor(Math.random() * 12); } while (idx2 === quizWord1.index);
    const quizWord2 = { index: idx2, word: words[idx2] };
    setState(chatId, 'QUIZ_2', { quizWord2 });
    await bot.sendMessage(chatId,
      `✅ Correct! What is word *#${quizWord2.index + 1}*?`,
      { parse_mode: 'Markdown' }
    );
    return;
  }

  // ── QUIZ word 2 answer ──
  if (s.step === 'QUIZ_2') {
    await safeDelete(chatId, msgId);
    const { quizWord2, mnemonicMsgId } = s.data;
    if (text.toLowerCase().trim() !== quizWord2.word.toLowerCase()) {
      await bot.sendMessage(chatId,
        `❌ That's not right. What is word *#${quizWord2.index + 1}*?`,
        { parse_mode: 'Markdown' }
      );
      return;
    }

    await bot.sendMessage(chatId, '✅ *Verified!* You saved your backup phrase.');

    setState(chatId, 'DELETE_CHOICE');
    await bot.sendMessage(chatId,
      'Would you like me to delete the backup phrase from this chat?',
      { parse_mode: 'Markdown', ...kb([
        [
          { text: 'Yes, delete it 🗑️', callback_data: 'MNEMONIC_DELETE_YES' },
          { text: 'No, keep it ⚠️',    callback_data: 'MNEMONIC_DELETE_NO'  }
        ]
      ])}
    );
    return;
  }

  // ── SEND: waiting for recipient ──
  if (s.step === 'SEND_TO') {
    setState(chatId, 'SEND_AMOUNT', { sendTo: text.toLowerCase() });
    await bot.sendMessage(chatId,
      `Send to: *${text.toLowerCase()}*\n\nEnter amount in HONE:`,
      { parse_mode: 'Markdown', ...kb([[{ text: '✕ Cancel', callback_data: 'MENU' }]]) }
    );
    return;
  }

  // ── SEND: waiting for amount ──
  // TODO: POST /api/bot/send does not exist on the server yet — needs server-side implementation
  if (s.step === 'SEND_AMOUNT') {
    const amount = parseFloat(text);
    if (isNaN(amount) || amount <= 0) {
      await bot.sendMessage(chatId,
        '❌ Invalid amount. Enter a number greater than 0:',
        kb([[{ text: '✕ Cancel', callback_data: 'MENU' }]])
      );
      return;
    }
    await bot.sendMessage(chatId,
      '⚠️ Sending HONE via Telegram is not available yet. Use the CLI or web wallet to send transactions.',
      { parse_mode: 'Markdown', ...kb([[{ text: '← Menu', callback_data: 'MENU' }]]) }
    );
    setState(chatId, 'MAIN_MENU');
    return;
  }

  // ── LINK: waiting for username ──
  if (s.step === 'LINK_USER') {
    const username = text.toLowerCase();
    setState(chatId, 'LINK_PASS', { linkUsername: username });
    await bot.sendMessage(chatId,
      `Enter your password for *${username}*:\n\n_Your message will be deleted immediately._`,
      { parse_mode: 'Markdown' }
    );
    return;
  }

  // ── LINK: waiting for password ──
  if (s.step === 'LINK_PASS') {
    await safeDelete(chatId, msgId);
    const { linkUsername } = s.data;
    const statusMsg = await bot.sendMessage(chatId, 'Linking account...');
    const data = await withRetryMsg(chatId, () =>
      apiCall('POST', '/login', { username: linkUsername, password: text, telegramId })
    );
    await safeDelete(chatId, statusMsg.message_id);
    if (!data) return;
    // Login response has no balance — fetch it separately
    const balData = await withRetryMsg(chatId, () =>
      apiCall('GET', '/balance', { telegramId })
    );
    const balance = balData ? balData.balance : 0;
    await bot.sendMessage(chatId,
      `🔗 ✅ Linked! This Telegram account is now connected to *${data.username}*.`,
      { parse_mode: 'Markdown' }
    );
    await showMainMenu(chatId, data.username, balance);
    return;
  }

  // ── Default: show menu if logged in, otherwise welcome ──
  const balData = await withRetryMsg(chatId, () =>
    apiCall('GET', '/balance', { telegramId })
  );
  if (balData) {
    await showMainMenu(chatId, balData.username, balData.balance);
  } else {
    await showWelcome(chatId);
  }
});

// ── Error handling ────────────────────────────────────────────────────────────
bot.on('polling_error', (err) => console.error('[honebot] Polling error:', err.code || err.message));
process.on('uncaughtException',  (err)    => console.error('[honebot] Uncaught:', err.message));
process.on('unhandledRejection', (reason) => console.error('[honebot] Unhandled:', reason));

console.log('[honebot] @honebot is live — conversational inline-keyboard UX');
