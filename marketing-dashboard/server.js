#!/usr/bin/env node
'use strict';

const express = require('express');
const cors    = require('cors');
const path    = require('path');
const fs      = require('fs/promises');
const crypto  = require('crypto');

const PORT       = process.env.MARKETING_PORT || 7979;
const DATA_DIR   = path.join(__dirname, 'data');
const QUEUE_FILE = path.join(DATA_DIR, 'queue.json');
const VOICE_FILE = path.join(DATA_DIR, 'voice_memory.json');
const SOUL_FILE  = path.join(__dirname, 'SOUL.md');

// ── env loading ────────────────────────────────────────────
function loadEnv() {
  try {
    const raw = require('fs').readFileSync(path.join(__dirname, '../.env'), 'utf8');
    const out = {};
    for (const line of raw.split('\n')) {
      const m = line.match(/^([A-Z_]+)=(.+)$/);
      if (m) out[m[1]] = m[2].trim().replace(/^["']|["']$/g, '');
    }
    return out;
  } catch { return {}; }
}
const ENV = loadEnv();
const HONE_API       = 'http://localhost:4242';
const HONE_KEY       = process.env.HONE_RELAY_API_KEY || ENV.HONE_RELAY_API_KEY || '';

// Try marketing .env first, fall back to honebot's real token
function loadBotToken() {
  if (process.env.TELEGRAM_BOT_TOKEN && process.env.TELEGRAM_BOT_TOKEN !== 'your-telegram-bot-token')
    return process.env.TELEGRAM_BOT_TOKEN;
  if (ENV.TELEGRAM_BOT_TOKEN && ENV.TELEGRAM_BOT_TOKEN !== 'your-telegram-bot-token')
    return ENV.TELEGRAM_BOT_TOKEN;
  try {
    const botEnv = require('fs').readFileSync(path.join(__dirname, '../../honebot/.env'), 'utf8');
    const m = botEnv.match(/^HONE_BOT_TOKEN=(.+)$/m);
    return m ? m[1].trim() : '';
  } catch { return ''; }
}
const TELEGRAM_TOKEN  = loadBotToken();
const TELEGRAM_CHAT   = process.env.TELEGRAM_CHAT_ID || ENV.TELEGRAM_CHAT_ID || '';

// ── per-platform disclosure text ───────────────────────────
const DISCLOSURES = {
  twitter:              '\n\n— AI-assisted, human-approved.',
  reddit_localllama:    '\n\n---\n*AI-assisted, human-approved.*',
  reddit_gpumining:     '\n\n---\n*AI-assisted, human-approved.*',
  reddit_cryptocurrency:'\n\n---\n*AI-assisted, human-approved.*',
  reddit_bitcoin:       '\n\n---\n*AI-assisted, human-approved.*',
  hn:                   '',
  substack:             '\n\n*Drafted with AI assistance, approved by a human.*',
};

function withDisclosure(content, platform) {
  return content + (DISCLOSURES[platform] || '');
}

// ── helpers ────────────────────────────────────────────────
async function readQueue() {
  try { return JSON.parse(await fs.readFile(QUEUE_FILE, 'utf8')); }
  catch { return []; }
}
async function writeQueue(posts) {
  await fs.writeFile(QUEUE_FILE, JSON.stringify(posts, null, 2));
}

async function readVoice() {
  try {
    const raw = JSON.parse(await fs.readFile(VOICE_FILE, 'utf8'));
    for (const p of ['shin', 'natoshi', 'josh']) {
      if (Array.isArray(raw[p])) raw[p] = { rules: [], raw: raw[p] };
      if (!raw[p]) raw[p] = { rules: [], raw: [] };
    }
    return raw;
  } catch {
    return {
      shin:    { rules: [], raw: [] },
      natoshi: { rules: [], raw: [] },
      josh:    { rules: [], raw: [] },
    };
  }
}
async function writeVoice(mem) {
  await fs.writeFile(VOICE_FILE, JSON.stringify(mem, null, 2));
}

// ── HONE inference (routes through chain → miners) ────────
async function honeInfer(messages, model = 'qwen3.5:9b') {
  if (!HONE_KEY) throw new Error('HONE_RELAY_API_KEY not set');
  const res = await fetch(`${HONE_API}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${HONE_KEY}` },
    body: JSON.stringify({ model, messages, temperature: 0.3 }),
  });
  if (!res.ok) throw new Error(`HONE inference ${res.status}: ${await res.text()}`);
  const data = await res.json();
  return data.choices[0].message.content.trim();
}

// ── Telegram notification ──────────────────────────────────
async function telegramNotify(text) {
  if (!TELEGRAM_TOKEN || TELEGRAM_TOKEN === 'your-telegram-bot-token') return;
  if (!TELEGRAM_CHAT) { console.warn('[telegram] TELEGRAM_CHAT_ID not set'); return; }
  try {
    await fetch(`https://api.telegram.org/bot${TELEGRAM_TOKEN}/sendMessage`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ chat_id: TELEGRAM_CHAT, text, parse_mode: 'Markdown' }),
    });
  } catch (e) { console.error('[telegram] notify failed:', e.message); }
}

// ── Quality self-check (async, doesn't block queue entry) ──
async function qualityCheck(post) {
  try {
    const soul = await fs.readFile(SOUL_FILE, 'utf8');
    const rules = soul.split('\n')
      .filter(l => l.startsWith('- ') || l.startsWith('* '))
      .slice(0, 20).join('\n');

    const result = await honeInfer([{ role: 'user', content:
      `You are reviewing a social post for the "${post.persona}" persona on ${post.platform}.

Key rules for this persona:
${rules}

Post to review:
"""
${post.content}
"""

Does this post violate any of the rules above? Reply with ONLY one of:
- PASS — if the post follows all rules
- FAIL: <specific rule violated> — if it breaks a rule

One line only.` }]);

    if (result.startsWith('FAIL')) {
      return result.replace('FAIL:', '').trim();
    }
    return null; // passed
  } catch (e) {
    console.error('[quality-check] failed:', e.message);
    return null; // don't block on inference failure
  }
}

// ── Voice memory distillation ──────────────────────────────
async function maybeDistil(persona, mem) {
  const raw = mem[persona].raw;
  if (raw.length < 5) return;
  try {
    const noteList = raw.map((n, i) =>
      `${i + 1}. [${n.platform}] Rejection: "${n.note}"\n   Example: "${n.excerpt}"`
    ).join('\n\n');

    const distilled = await honeInfer([{ role: 'user', content:
      `You are refining the voice of the "${persona}" persona for HONE marketing.

These are ${raw.length} rejection notes — why specific posts were rejected:

${noteList}

Write 3–6 concise, specific rules about what "${persona}" must NEVER do in posts.
Capture lasting patterns, not one-off complaints. Be direct and actionable.
Format: one rule per line, no numbers, no preamble.` }]);

    const newRules = distilled.split('\n').map(l => l.trim()).filter(Boolean);

    let finalRules;
    if (mem[persona].rules.length > 0) {
      const merged = await honeInfer([{ role: 'user', content:
        `Merge these voice rules for the "${persona}" HONE marketing persona.

Existing rules:
${mem[persona].rules.map(r => `- ${r}`).join('\n')}

New rules to merge:
${newRules.map(r => `- ${r}`).join('\n')}

Produce a deduplicated final list (max 10). Keep the most specific version of any duplicate.
Output one rule per line, no numbers, no preamble.` }]);
      finalRules = merged.split('\n').map(l => l.trim()).filter(Boolean).slice(0, 10);
    } else {
      finalRules = newRules.slice(0, 10);
    }

    mem[persona].rules = finalRules;
    mem[persona].raw   = [];
    await writeVoice(mem);
    console.log(`[voice-memory] Distilled ${raw.length} notes → ${finalRules.length} rules for ${persona}`);
  } catch (err) {
    console.error(`[voice-memory] Distillation failed for ${persona}:`, err.message);
  }
}

// ── Express app ────────────────────────────────────────────
const app = express();
app.use(cors());
app.use(express.json({ limit: '2mb' }));
app.use(express.static(__dirname));

// GET /api/posts?status=&persona=&for_posting=1
// for_posting=1 appends disclosure text to content — used by ZeroClaw poster
app.get('/api/posts', async (req, res) => {
  let posts = await readQueue();
  if (req.query.status)  posts = posts.filter(p => p.status  === req.query.status);
  if (req.query.persona) posts = posts.filter(p => p.persona === req.query.persona);
  if (req.query.for_posting === '1') {
    posts = posts.map(p => ({ ...p, content: withDisclosure(p.content, p.platform) }));
  }
  res.json(posts);
});

// POST /api/posts — ZeroClaw drafter submits new drafts
app.post('/api/posts', async (req, res) => {
  const posts = await readQueue();
  const post = {
    id:          req.body.id || crypto.randomUUID(),
    persona:     req.body.persona,
    platform:    req.body.platform,
    content:     req.body.content,
    status:      'draft',
    pillar:      req.body.pillar || null,
    notes:       req.body.notes || '',
    scheduledAt: req.body.scheduledAt || null,
    createdAt:   new Date().toISOString(),
    approvedAt:  null,
    postedAt:    null,
    qualityWarning: null,
  };
  posts.push(post);
  await writeQueue(posts);

  // Async quality check — updates the post's qualityWarning if it fails
  setImmediate(async () => {
    const warning = await qualityCheck(post);
    if (warning) {
      const all = await readQueue();
      const idx = all.findIndex(p => p.id === post.id);
      if (idx !== -1) { all[idx].qualityWarning = warning; await writeQueue(all); }
      console.log(`[quality] WARNING for ${post.persona}/${post.platform}: ${warning}`);
    }
  });

  res.status(201).json(post);
});

// PATCH /api/posts/:id
app.patch('/api/posts/:id', async (req, res) => {
  const posts = await readQueue();
  const idx = posts.findIndex(p => p.id === req.params.id);
  if (idx === -1) return res.status(404).json({ error: 'not found' });

  const prev = posts[idx];
  const next = { ...prev, ...req.body };

  if (req.body.status === 'approved') {
    next.approvedAt = new Date().toISOString();
    delete next.rejectionNote;
    next.qualityWarning = null; // human approved — clear the warning
  }
  if (req.body.status === 'posted') {
    next.postedAt = new Date().toISOString();
  }
  if (req.body.status === 'failed') {
    // ZeroClaw couldn't post — notify Josh via Telegram
    const msg = `⚠️ *Post failed to publish*\n\nPersona: ${prev.persona}\nPlatform: ${prev.platform}\nReason: ${req.body.failReason || 'unknown'}\n\nContent preview:\n_${prev.content.slice(0, 100)}..._\n\nPost ID: \`${prev.id}\``;
    setImmediate(() => telegramNotify(msg));
  }
  if (req.body.status === 'draft' && req.body.rejectionNote) {
    const mem = await readVoice();
    const persona = prev.persona;
    if (mem[persona]) {
      mem[persona].raw.push({
        note:        req.body.rejectionNote,
        excerpt:     prev.content.slice(0, 200),
        platform:    prev.platform,
        rejected_at: new Date().toISOString(),
      });
      await writeVoice(mem);
      if (mem[persona].raw.length >= 5) {
        setImmediate(() => maybeDistil(persona, mem));
      }
    }
  }

  posts[idx] = next;
  await writeQueue(posts);
  res.json(next);
});

// DELETE /api/posts/:id
app.delete('/api/posts/:id', async (req, res) => {
  const posts = await readQueue();
  const idx = posts.findIndex(p => p.id === req.params.id);
  if (idx === -1) return res.status(404).json({ error: 'not found' });
  posts.splice(idx, 1);
  await writeQueue(posts);
  res.status(204).end();
});

// ── voice memory ───────────────────────────────────────────
app.get('/api/voice-memory', async (req, res) => res.json(await readVoice()));
app.get('/api/voice-memory/:persona', async (req, res) => {
  const mem = await readVoice();
  res.json(mem[req.params.persona] || { rules: [], raw: [] });
});

// ── SOUL.md ────────────────────────────────────────────────
app.get('/api/soul', async (req, res) => {
  try { res.type('text/plain').send(await fs.readFile(SOUL_FILE, 'utf8')); }
  catch { res.status(404).json({ error: 'SOUL.md not found' }); }
});

// ── bulk import ────────────────────────────────────────────
app.post('/api/import', async (req, res) => {
  if (!Array.isArray(req.body.posts)) return res.status(400).json({ error: 'posts must be array' });
  await writeQueue(req.body.posts);
  res.json({ imported: req.body.posts.length });
});

app.listen(PORT, () => {
  console.log(`HONE Marketing Dashboard → http://localhost:${PORT}`);
  if (!HONE_KEY)      console.warn('  WARNING: HONE_RELAY_API_KEY not set — quality checks and distillation disabled');
  if (!TELEGRAM_TOKEN || TELEGRAM_TOKEN === 'your-telegram-bot-token')
    console.warn('  WARNING: TELEGRAM_BOT_TOKEN not set — failed-post notifications disabled');
  if (!TELEGRAM_CHAT)  console.warn('  WARNING: TELEGRAM_CHAT_ID not set — failed-post notifications disabled');
});
