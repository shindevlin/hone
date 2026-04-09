#!/usr/bin/env node
"use strict";

/**
 * BTCPC Project Runner — Intelligent inference for all 12 projects
 * Shin Devlin
 *
 * Each project gets a cron cycle that:
 * 1. Reads past inference results (.btcpc-inference/)
 * 2. Analyzes what's been done and what's needed
 * 3. Generates contextual new prompts based on project purpose
 * 4. Submits inference jobs via BTCPC API
 * 5. Saves results back to the project
 * 6. Tracks quality metrics
 *
 * Runs continuously — each project gets a turn every 2 minutes.
 * Daily quality audit runs at midnight.
 */

const fs = require('fs');
const path = require('path');
const axios = require('axios');
const crypto = require('crypto');

// ─── Configuration ───────────────────────────────────────────────

const INTERVAL_MS = parseInt(process.env.JOB_INTERVAL_MS) || 120000;
const API_URL = 'http://localhost:3100';
const REPOS_DIR = '/home/ubuntclaw/repos';

// Project definitions with intelligent prompt strategies
const PROJECTS = {
  bullship: {
    dir: 'bullship',
    purpose: 'Crypto news trivia game — fake but believable headlines',
    strategy: 'rotating',
    prompts: {
      generate: [
        "Write a fake but completely believable crypto news headline and a 2-sentence article excerpt. Topic: {topic}. Make it sound like it's from Reuters or Bloomberg. Include a specific price, date, and company name.",
        "Create a breaking crypto news alert that sounds 100% real. Include: headline, source attribution, key quote from an analyst, and market impact. Topic: {topic}.",
        "Write a satirical but plausible crypto headline that could fool someone for 5 seconds. Topic: {topic}. Include a realistic subheadline."
      ],
      topics: ['Bitcoin ETF', 'Ethereum upgrade', 'Solana outage', 'memecoin surge', 'DeFi exploit', 'regulatory action', 'stablecoin depeg', 'mining ban', 'NFT comeback', 'AI token launch'],
      analyze: "Review these past headlines and suggest 3 new topics we haven't covered yet that would be trending in crypto news this week: {past_results}"
    }
  },

  nsfwotica: {
    dir: 'nsfwotica',
    purpose: 'Accessible story platform for disabled users — rich sensory writing',
    strategy: 'sequential',
    prompts: {
      generate: [
        "Write a 500-word short story chapter with rich sensory description (sounds, textures, temperatures, scents). Setting: {setting}. Focus on accessibility — describe scenes so a screen reader conveys the full atmosphere. Include dialogue.",
        "Continue a story: the protagonist has just {event}. Write the next scene with vivid audio and tactile descriptions. 400 words. Include internal monologue.",
        "Write a slice-of-life scene in {setting}. Focus on: ambient sounds, fabric textures, food aromas, warm/cool sensations. Make a reader who can't see feel present in the scene."
      ],
      settings: ['a coastal Italian village at dawn', 'a Tokyo ramen shop in rain', 'a Marrakech night market', 'a Swedish cabin during a snowstorm', 'a Buenos Aires tango hall', 'a Kyoto temple garden in autumn'],
      events: ['arrived in a new city with only a backpack', 'received a letter from someone they thought was gone', 'found an old key hidden in a library book', 'heard a melody that unlocked a forgotten memory']
    }
  },

  brutus11: {
    dir: 'brutus11',
    purpose: 'Ohio State football news — analysis, previews, commentary',
    strategy: 'topical',
    prompts: {
      generate: [
        "Write a 400-word analysis piece about Ohio State football. Topic: {topic}. Include specific stats, player names, and historical context. Write in the style of a respected sports columnist.",
        "Create an Ohio State football power ranking argument. Why should the Buckeyes be ranked {rank} this week? Include strength of schedule, key wins, and comparison to rivals.",
        "Write a scouting report on Ohio State's {position} group. Evaluate 3 specific players with stats, strengths, weaknesses, and NFL draft projection."
      ],
      topics: ['2026 recruiting class analysis', 'Big Ten conference realignment impact', 'NIL strategy vs Michigan', 'defensive scheme evolution under new coordinator', 'quarterback battle preview', 'rivalry week preparation'],
      ranks: ['#1', '#3', '#5', '#8', '#12'],
      positions: ['quarterback', 'wide receiver', 'offensive line', 'linebacker', 'defensive back', 'defensive line']
    }
  },

  ursos: {
    dir: 'ursOS',
    purpose: 'System admin assistant — Linux tutorials and troubleshooting',
    strategy: 'knowledge_base',
    prompts: {
      generate: [
        "Write a comprehensive tutorial: {topic}. Include: prerequisites, step-by-step commands (with explanations), common pitfalls, and verification steps. Target audience: intermediate Linux admin.",
        "Troubleshooting guide: {problem}. Include: symptoms, diagnostic commands, root cause analysis, fix steps, and prevention measures.",
        "Compare and contrast: {comparison}. Include a decision matrix with pros/cons, performance characteristics, and when to use each."
      ],
      topics: ['Setting up WireGuard VPN on Ubuntu 24.04', 'Docker Compose networking for microservices', 'Automated backup with restic and systemd timers', 'Setting up Fail2Ban with custom rules', 'ZFS pool management and snapshots', 'Nginx reverse proxy with SSL termination'],
      problems: ['Container can\'t resolve DNS after Docker update', 'SystemD service fails silently on boot', 'Disk I/O bottleneck on database server', 'SSH connection drops after 5 minutes of inactivity', 'OOM killer targeting the wrong process'],
      comparisons: ['ZFS vs Btrfs for a NAS', 'Podman vs Docker for production', 'UFW vs nftables for server firewall', 'Caddy vs Nginx for reverse proxy']
    }
  },

  realfake: {
    dir: 'realfake',
    purpose: 'AI character roleplay — distinct personalities and voices',
    strategy: 'character',
    prompts: {
      generate: [
        "You are {character}. {scenario}. Stay completely in character. Write 300-400 words in first person. Include mannerisms, speech patterns, and period-appropriate vocabulary.",
        "Two characters meet: {character1} encounters {character2} at {location}. Write their dialogue (300 words). Each character must have a distinct voice and perspective.",
        "You are {character}. Write your diary entry for today. Include: what happened, how you feel about it, a philosophical reflection, and plans for tomorrow. 400 words."
      ],
      characters: ['a Victorian detective obsessed with order', 'an AI that just became self-aware', 'the last librarian on Earth', 'a Roman senator during the fall of the Republic', 'a Mars colony botanist in 2187', 'a 1920s jazz musician in Harlem'],
      scenarios: ['You\'ve just discovered evidence that changes everything you believed', 'A stranger has asked you the one question you\'ve been avoiding', 'You must make a decision that will affect everyone around you', 'You\'re writing a letter to someone you\'ll never see again'],
      locations: ['a midnight bookshop', 'a train crossing the Sahara', 'the last restaurant on Earth', 'a space station observation deck']
    }
  },

  redaktly: {
    dir: 'redaktly',
    purpose: 'PII detection and document redaction',
    strategy: 'detection',
    prompts: {
      generate: [
        "Generate a realistic business document that contains the following PII types: {pii_types}. The document should be a {doc_type}. Make it realistic — use plausible names, addresses, numbers. Then identify and list ALL PII found with exact positions.",
        "Review this text for PII and sensitive data. Categorize each finding as: Critical (SSN, financial), High (medical, legal), Medium (address, phone), Low (name, email). Text: {sample_text}",
        "Create a GDPR/CCPA compliance checklist for this document type: {doc_type}. List what PII is expected, what must be redacted for sharing, and what retention rules apply."
      ],
      pii_types: ['SSN + address + DOB', 'medical records + insurance ID', 'bank account + routing number', 'passport + visa number', 'employee ID + salary + performance review'],
      doc_types: ['employment contract', 'medical intake form', 'insurance claim', 'legal deposition transcript', 'bank loan application', 'real estate closing document']
    }
  },

  buswingspread: {
    dir: 'BusWingSpread',
    purpose: 'Business analysis — AI-proof service businesses with acquisition potential',
    strategy: 'research',
    prompts: {
      generate: [
        "Analyze the acquisition potential of a {business_type} business in {location}. Include: market size, typical margins, AI disruption risk (low/medium/high), valuation multiples, and key success factors. 500 words.",
        "Write a SWOT analysis for: {scenario}. Include 3 specific items per category with supporting data or reasoning. Conclude with a strategic recommendation.",
        "Create a competitive landscape map for {industry} in {market}. Identify 5 competitor types, their positioning, pricing tiers, and the underserved gap a new entrant could fill."
      ],
      business_types: ['commercial cleaning', 'mobile auto detailing', 'property management', 'specialty food truck', 'B2B IT support', 'elder care coordination'],
      locations: ['suburban Dallas', 'downtown Portland', 'rural Tennessee', 'Miami Beach', 'Boise Idaho', 'Austin Texas'],
      industries: ['home services', 'pet care', 'commercial real estate', 'healthcare staffing', 'specialty food retail']
    }
  },

  'betchu-bot': {
    dir: 'betchu_bot',
    purpose: 'Sports betting analysis — odds, matchups, value identification',
    strategy: 'analytical',
    prompts: {
      generate: [
        "Analyze this matchup for betting value: {matchup}. Consider: recent form, head-to-head history, injuries, venue advantage, and market odds. Identify the value bet with reasoning. Include a confidence level (1-10).",
        "Create a {sport} parlay analysis for this weekend. Pick 3 legs with: selection, odds, reasoning, and risk assessment. Calculate combined odds and expected value.",
        "Write a contrarian take: why the {underdog} will upset {favorite} in {event}. Use statistical backing, not just narrative. Include: key metric, historical precedent, and situational advantage."
      ],
      matchups: ['Lakers vs Celtics NBA regular season', 'Liverpool vs Real Madrid Champions League', 'Packers vs Cowboys NFL Week 8', 'Djokovic vs Alcaraz Grand Slam final'],
      sports: ['NFL', 'NBA', 'MLB', 'Premier League', 'Champions League'],
      underdogs: ['16-seed March Madness team', 'promoted Premier League side', 'wild card MLB team', 'expansion MLS team']
    }
  },

  counselflow: {
    dir: 'counselflow',
    purpose: 'Legal bridge — American procedures for international attorneys',
    strategy: 'legal',
    prompts: {
      generate: [
        "Explain {procedure} in American law for an attorney from {country} who is unfamiliar with the US legal system. Include: step-by-step process, key forms/documents, typical timeline, common mistakes foreign attorneys make, and how it differs from {country}'s approach.",
        "Draft a template for: {document}. Include all required sections, standard language, and annotations explaining each section's purpose. Note jurisdiction-specific variations.",
        "Compare {concept} in American law vs {country} law. Include: terminology differences, procedural differences, enforcement differences, and practical implications for cross-border cases."
      ],
      procedures: ['filing a civil complaint in federal court', 'discovery process and depositions', 'motion for summary judgment', 'Chapter 11 bankruptcy filing', 'patent application process'],
      countries: ['Brazil', 'Germany', 'Japan', 'Nigeria', 'India', 'South Korea', 'Mexico'],
      documents: ['cease and desist letter', 'NDA for cross-border deal', 'LLC operating agreement', 'commercial lease agreement']
    }
  },

  'spirit-of-ngu': {
    dir: 'spirit-of-ngu',
    purpose: 'Idle game — satirical content, lore, dialogue, achievements',
    strategy: 'creative',
    prompts: {
      generate: [
        "Write 5 satirical idle game achievements with names and descriptions. Theme: {theme}. Each should be funny, reference crypto/tech culture, and have a hidden double meaning. Format: Name: description (unlock condition).",
        "Create boss fight dialogue for the Idleverse guardian of the {zone} zone. Include: entrance monologue, 3 attack taunts, a defeat speech, and a secret post-defeat easter egg line. Tone: satirical but epic.",
        "Write {count} loading screen tips for an idle game. Mix: genuinely useful game tips, meta-commentary on idle games, crypto in-jokes, and absurdist humor. Each tip should be 1-2 sentences."
      ],
      themes: ['DeFi yield farming', 'GPU mining rigs', 'VC pitch meetings', 'Discord moderating', 'blockchain consensus', 'meme coin trading'],
      zones: ['Tutorial Plains', 'Hashrate Highlands', 'Liquidity Lake', 'Governance Gorge', 'Whale Watch Tower', 'Rug Pull Ravine'],
      counts: [10, 15, 20]
    }
  },

  itsurs: {
    dir: 'itsurs',
    purpose: 'Social media platform — content moderation, recommendations, creator tools',
    strategy: 'platform',
    prompts: {
      generate: [
        "Analyze this social media post for content policy compliance. Identify any issues (harassment, misinformation, spam, copyright). Post: \"{post}\". Provide: ruling (allow/flag/remove), reasoning, confidence level, and suggested action.",
        "Generate 5 hashtag suggestions and a caption for a post about: {topic}. Target audience: {audience}. Tone: {tone}. Include engagement hooks.",
        "Write a content strategy brief for a creator in the {niche} space. Include: posting frequency, content pillars (3-5), engagement tactics, growth metrics to track, and monetization opportunities."
      ],
      posts: ['Just made $50K from my new crypto course! DM me to learn how (not financial advice)', 'This politician is literally destroying our country and should be removed by any means', 'Check out my original artwork! [actually AI-generated]'],
      topics: ['sustainable fashion', 'home cooking hacks', 'indie game development', 'urban gardening', 'AI art'],
      niches: ['fitness coaching', 'tech reviews', 'travel vlogging', 'personal finance', 'DIY crafts']
    }
  },

  waitlyfi: {
    dir: 'waitlyfi',
    purpose: 'Waitlist platform — email copy, landing page content, conversion optimization',
    strategy: 'conversion',
    prompts: {
      generate: [
        "Write a complete email sequence for a {product_type} waitlist. Include: 1) Confirmation email, 2) Week 1 update, 3) Access granted email. Each should have: subject line, preview text, body, and CTA. Tone: {tone}.",
        "Create 5 headline variations for a {product_type} waitlist landing page. Each should communicate value, create urgency, and be under 10 words. Include a supporting subheadline for each.",
        "Write microcopy for a waitlist widget: placeholder text, submit button, success message, error message, social proof line, and referral prompt. Product: {product_type}. Tone: {tone}."
      ],
      product_types: ['fintech savings app', 'AI writing assistant', 'developer tool', 'health tracking wearable', 'crypto portfolio manager', 'remote work platform'],
      tones: ['playful and bold', 'professional and trustworthy', 'minimalist and premium', 'urgent and exclusive']
    }
  }
};

// ─── Prompt Generation ───────────────────────────────────────────

function pickRandom(arr) {
  return arr[Math.floor(Math.random() * arr.length)];
}

function fillTemplate(template, project) {
  return template.replace(/\{(\w+)\}/g, (_, key) => {
    if (project.prompts[key]) return pickRandom(project.prompts[key]);
    if (project.prompts[key + 's']) return pickRandom(project.prompts[key + 's']);
    if (key.endsWith('s') && project.prompts[key.slice(0, -1)]) return pickRandom(project.prompts[key.slice(0, -1)]);
    var base = key.replace(/\d+$/, '');
    if (base !== key && project.prompts[base + 's']) return pickRandom(project.prompts[base + 's']);
    return `[${key}]`;
  });
}

function generatePrompt(projectName, project) {
  const templates = project.prompts.generate;
  const template = pickRandom(templates);
  return fillTemplate(template, project);
}

// ─── Past Results Analysis ───────────────────────────────────────

function getPastResults(projectDir, limit) {
  const inferDir = path.join(REPOS_DIR, projectDir, '.btcpc-inference');
  if (!fs.existsSync(inferDir)) return [];

  return fs.readdirSync(inferDir)
    .filter(f => f.endsWith('.json'))
    .sort().reverse().slice(0, limit || 5)
    .map(f => {
      try { return JSON.parse(fs.readFileSync(path.join(inferDir, f))); }
      catch (_) { return null; }
    })
    .filter(Boolean);
}

// ─── Job Submission ──────────────────────────────────────────────

async function submitJob(projectName, project, prompt) {
  const envPath = path.join(REPOS_DIR, project.dir, '.envbtcpc');
  if (!fs.existsSync(envPath)) return null;

  const env = {};
  fs.readFileSync(envPath, 'utf8').split('\n').forEach(line => {
    const [k, ...v] = line.split('=');
    if (k && v.length) env[k.trim()] = v.join('=').trim();
  });

  if (!env.BTCPC_PROJECT_KEY) return null;

  try {
    const { data } = await axios.post(`${API_URL}/v1/inference/submit`, {
      model: 'auto',
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 1024
    }, {
      timeout: 15000,
      headers: { 'Authorization': `Bearer ${env.BTCPC_PROJECT_KEY}` }
    });

    return data;
  } catch (err) {
    console.error(`[${projectName}] Submit failed: ${err.response?.data?.error?.message || err.message}`);
    return null;
  }
}

async function pollResult(projectName, project, jobId) {
  const envPath = path.join(REPOS_DIR, project.dir, '.envbtcpc');
  const env = {};
  fs.readFileSync(envPath, 'utf8').split('\n').forEach(line => {
    const [k, ...v] = line.split('=');
    if (k && v.length) env[k.trim()] = v.join('=').trim();
  });

  const maxWait = 300000;
  const start = Date.now();

  while (Date.now() - start < maxWait) {
    try {
      const { data } = await axios.get(`${API_URL}/v1/inference/${jobId}`, {
        headers: { 'Authorization': `Bearer ${env.BTCPC_PROJECT_KEY}` }
      });
      if (data.status === 'completed' || data.status === 'settled') return data;
      if (data.status === 'failed') return null;
    } catch (_) {}
    await new Promise(r => setTimeout(r, 5000));
  }
  return null;
}

function saveResult(projectName, project, jobId, prompt, result) {
  const inferDir = path.join(REPOS_DIR, project.dir, '.btcpc-inference');
  if (!fs.existsSync(inferDir)) fs.mkdirSync(inferDir, { recursive: true });

  const output = {
    job_id: jobId,
    project: projectName,
    prompt,
    model: result.model || 'auto',
    tokens: result.tokens_generated || result.tokens || 0,
    result: result.result_text || result.result?.content || '',
    timestamp: new Date().toISOString(),
    quality_score: null // filled by daily audit
  };

  fs.writeFileSync(path.join(inferDir, `${jobId}.json`), JSON.stringify(output, null, 2));
  return output;
}

// ─── Main Loop ───────────────────────────────────────────────────

const projectNames = Object.keys(PROJECTS);
let currentIndex = 0;
let totalJobs = 0;
let totalTokens = 0;

async function runCycle() {
  const name = projectNames[currentIndex];
  const project = PROJECTS[name];
  const ts = new Date().toISOString();

  // Generate contextual prompt
  const prompt = generatePrompt(name, project);

  // Submit
  const job = await submitJob(name, project, prompt);
  if (!job) {
    currentIndex = (currentIndex + 1) % projectNames.length;
    return;
  }

  totalJobs++;
  console.log(`[${ts}] #${totalJobs} ${name}: ${job.job_id} (${job.model})`);
  console.log(`  Prompt: ${prompt.slice(0, 80)}...`);

  // Poll for result
  setTimeout(async () => {
    const result = await pollResult(name, project, job.job_id);
    if (result) {
      const saved = saveResult(name, project, job.job_id, prompt, result);
      const tokens = saved.tokens || 0;
      totalTokens += tokens;
      console.log(`[${new Date().toISOString()}] Saved ${name}/${job.job_id}.json (${tokens} tokens, total: ${totalTokens})`);
    }
  }, 10000);

  currentIndex = (currentIndex + 1) % projectNames.length;
}

// ─── Quality Audit (daily) ───────────────────────────────────────

async function dailyQualityAudit() {
  console.log(`[AUDIT] Starting daily quality audit...`);

  for (const [name, project] of Object.entries(PROJECTS)) {
    const results = getPastResults(project.dir, 10);
    if (results.length === 0) continue;

    const summaries = results.slice(0, 5).map(r =>
      `Prompt: ${(r.prompt || '').slice(0, 60)}\nResult: ${(r.result || '').slice(0, 100)}`
    ).join('\n---\n');

    const auditPrompt = `You are a quality auditor for AI inference results. Rate each result 1-10 for: relevance, accuracy, usefulness, and writing quality. Project purpose: ${project.purpose}\n\nResults to audit:\n${summaries}\n\nRespond with a JSON array of scores and one-line feedback per result.`;

    // Submit audit as an inference job from the first available project
    try {
      const auditJob = await submitJob(name, project, auditPrompt);
      if (auditJob) {
        console.log(`[AUDIT] ${name}: audit submitted (${auditJob.job_id})`);
      }
    } catch (_) {}
  }

  console.log(`[AUDIT] Daily audit complete.`);
}

// ─── Start ───────────────────────────────────────────────────────

console.log(`[project-runner] ${projectNames.length} projects loaded, interval: ${INTERVAL_MS / 1000}s`);
console.log(`[project-runner] Projects: ${projectNames.join(', ')}`);
projectNames.forEach(n => {
  const p = PROJECTS[n];
  const envPath = path.join(REPOS_DIR, p.dir, '.envbtcpc');
  const hasKey = fs.existsSync(envPath) && fs.readFileSync(envPath, 'utf8').includes('BTCPC_PROJECT_KEY');
  console.log(`  ${n}: ${hasKey ? 'ready' : 'NO API KEY'} (${p.purpose.slice(0, 50)})`);
});

// Run first cycle immediately
runCycle();

// Then every INTERVAL_MS
setInterval(runCycle, INTERVAL_MS);

// Daily audit at midnight
const now = new Date();
const midnight = new Date(now);
midnight.setHours(24, 0, 0, 0);
const msUntilMidnight = midnight - now;
setTimeout(() => {
  dailyQualityAudit();
  setInterval(dailyQualityAudit, 24 * 60 * 60 * 1000);
}, msUntilMidnight);

console.log(`[project-runner] Daily audit scheduled in ${Math.round(msUntilMidnight / 60000)} minutes`);
