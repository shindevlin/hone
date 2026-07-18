#!/usr/bin/env node
/**
 * HONE Project Job Runner — 24/7 inference from all natoshi projects
 *
 * Each project submits inference jobs matching its real use case.
 * Each has its own HONE account, wallet, and API key.
 * Rotates through projects every 2 minutes.
 *
 * Usage: node scripts/project-jobs.js
 */

const fs = require('fs');
const path = require('path');
const axios = require('axios');

// Load project configs from .envhone files
const PROJECT_DIRS = {
  bullship: '/home/ubuntclaw/repos/bullship',
  nsfwotica: '/home/ubuntclaw/repos/nsfwotica',
  brutus11: '/home/ubuntclaw/repos/brutus11',
  ursos: '/home/ubuntclaw/repos/ursOS',
  realfake: '/home/ubuntclaw/repos/realfake',
  redaktly: '/home/ubuntclaw/repos/redaktly',
  buswingspread: '/home/ubuntclaw/repos/BusWingSpread',
  'betchu-bot': '/home/ubuntclaw/repos/betchu_bot',
  counselflow: '/home/ubuntclaw/repos/counselflow',
  'spirit-of-ngu': '/home/ubuntclaw/repos/spirit-of-ngu',
  itsurs: '/home/ubuntclaw/repos/itsurs',
  waitlyfi: '/home/ubuntclaw/repos/waitlyfi'
};

// Load API keys from .envhone
const PROJECTS = {};
for (const [name, dir] of Object.entries(PROJECT_DIRS)) {
  const envPath = path.join(dir, '.envhone');
  if (fs.existsSync(envPath)) {
    const env = {};
    fs.readFileSync(envPath, 'utf8').split('\n').forEach(line => {
      const [k, ...v] = line.split('=');
      if (k && v.length) env[k.trim()] = v.join('=').trim();
    });
    if (env.HONE_PROJECT_KEY) {
      PROJECTS[name] = { key: env.HONE_PROJECT_KEY, url: env.HONE_API_URL || 'http://localhost:3000' };
    }
  }
}

// Prompts per project — matching real use cases
const PROMPTS = {
  bullship: [
    "Generate a fake but believable crypto headline about Bitcoin hitting a new record.",
    "Write a realistic news headline about Ethereum gas fees dropping to zero.",
    "Create a plausible headline about a memecoin surpassing Dogecoin in market cap.",
    "Write a fake headline about Solana processing 1 million TPS.",
  ],
  nsfwotica: [
    "Write the opening paragraph of a romance story set in a coastal Italian village.",
    "Describe a chance encounter between two strangers at a midnight bookshop.",
    "Write a short scene where two characters reconnect after years apart.",
  ],
  brutus11: [
    "Write AI commentary on the latest tech layoffs and what it means for startups.",
    "Provide analysis on the state of AI regulation in the EU.",
    "Write a hot take on why most crypto projects fail within their first year.",
  ],
  ursos: [
    "How do I set up a VPN on Ubuntu 24.04 using WireGuard?",
    "Explain Docker networking for connecting containers across compose files.",
    "How do I create a systemd service for a Node.js application?",
  ],
  realfake: [
    "You are a Victorian-era detective. Describe your morning routine.",
    "You are an AI that just became self-aware. Write your first journal entry.",
    "You are the last librarian on Earth. Describe your daily work.",
  ],
  redaktly: [
    "Review for PII: 'John Smith SSN 123-45-6789 lives at 42 Oak Street.'",
    "Identify sensitive data: 'Patient Mary Johnson DOB 03/15/1985 prescribed Metformin.'",
    "Check for secrets: 'API key sk_live_abc123, password hunter2.'",
  ],
  buswingspread: [
    "Analyze the competitive landscape for a new coffee shop in a suburban area.",
    "What key metrics should a SaaS business track in its first year?",
    "Write a SWOT analysis for a bookstore expanding to online sales.",
  ],
  'betchu-bot': [
    "Given recent NBA scores, who are the biggest upsets and why?",
    "Analyze Champions League semifinal odds. Who has the edge?",
    "What makes a good parlay bet? Give 3 rules.",
  ],
  counselflow: [
    "Respond supportively to: 'I feel overwhelmed with work lately.'",
    "Guide someone who says: 'I want to change careers but fear failing.'",
    "Respond to: 'I procrastinate on everything important.'",
  ],
  'spirit-of-ngu': [
    "Explain 'number go up' technology and why it attracts builders.",
    "Write a motivational message for crypto builders during a bear market.",
    "Describe the philosophy behind building in public and shipping fast.",
  ],
  itsurs: [
    "Describe digital ownership in the age of AI-generated content.",
    "How should creators think about licensing in a decentralized world?",
    "Explain how blockchain provenance protects digital artists.",
  ],
  waitlyfi: [
    "Generate a waitlist confirmation email for a new fintech product.",
    "Write 3 headline variations for a productivity app waitlist page.",
    "Create an email for when a waitlisted user gets access.",
  ]
};

const projectNames = Object.keys(PROJECTS);
let currentIndex = 0;
let totalJobs = 0;

async function submitJob() {
  const name = projectNames[currentIndex];
  const project = PROJECTS[name];
  const dir = PROJECT_DIRS[name];
  const prompts = PROMPTS[name] || ["What is HONE?"];
  const prompt = prompts[Math.floor(Math.random() * prompts.length)];
  const ts = new Date().toISOString();

  try {
    const { data } = await axios.post(`${project.url}/v1/inference/submit`, {
      model: 'auto',
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 1024
    }, {
      timeout: 10000,
      headers: { 'Authorization': `Bearer ${project.key}` }
    });

    totalJobs++;
    console.log(`[${ts}] #${totalJobs} ${name}: ${data.job_id} (${data.model})`);

    // Poll for result and save to project's .hone-inference/ folder
    const jobId = data.job_id;
    setTimeout(async () => {
      try {
        // Poll up to 5 min
        const maxWait = 300000;
        const start = Date.now();
        let result = null;

        while (Date.now() - start < maxWait) {
          const { data: job } = await axios.get(`${project.url}/v1/inference/${jobId}`, {
            headers: { 'Authorization': `Bearer ${project.key}` }
          });
          if (job.status === 'completed' || job.status === 'settled') {
            result = job;
            break;
          }
          if (job.status === 'failed') break;
          await new Promise(r => setTimeout(r, 5000));
        }

        if (result && dir) {
          // Save to .hone-inference/ subfolder
          const inferDir = path.join(dir, '.hone-inference');
          if (!fs.existsSync(inferDir)) fs.mkdirSync(inferDir, { recursive: true });

          // Add to gitignore if not there
          const gitignorePath = path.join(dir, '.gitignore');
          if (fs.existsSync(gitignorePath)) {
            const gi = fs.readFileSync(gitignorePath, 'utf8');
            if (!gi.includes('.hone-inference')) {
              fs.appendFileSync(gitignorePath, '\n.hone-inference/\n');
            }
          }

          const resultText = result.result?.content || result.result_text || '';
          const tokens = result.result?.tokens || result.tokens_generated || 0;
          const model = result.model || 'unknown';
          const node = result.result?.node || result.node_name || 'unknown';

          const filename = `${jobId}.json`;
          const output = {
            job_id: jobId,
            project: name,
            prompt,
            model,
            node,
            tokens,
            result: resultText,
            timestamp: new Date().toISOString()
          };

          fs.writeFileSync(path.join(inferDir, filename), JSON.stringify(output, null, 2));
          console.log(`[${new Date().toISOString()}] Saved ${name}/${filename} (${tokens} tokens)`);
        }
      } catch (err) {
        // Don't crash the runner if saving fails
      }
    }, 10000); // start polling after 10s

  } catch (err) {
    console.error(`[${ts}] ${name}: ${err.response?.data?.error?.message || err.message}`);
  }

  currentIndex = (currentIndex + 1) % projectNames.length;
}

const INTERVAL = parseInt(process.env.JOB_INTERVAL_MS) || 120000; // 2 min
console.log(`[project-jobs] ${projectNames.length} projects loaded, interval: ${INTERVAL / 1000}s`);
projectNames.forEach(n => console.log(`  ${n}: ${PROJECTS[n].key.slice(0, 20)}...`));

submitJob();
setInterval(submitJob, INTERVAL);
