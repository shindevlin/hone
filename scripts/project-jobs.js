#!/usr/bin/env node
/**
 * BTCPC Project Job Runner
 *
 * Submits real inference jobs for each natoshisakamoto project.
 * Each project gets prompts matching its actual use case.
 * Rotates through projects on a timer.
 *
 * Usage: node scripts/project-jobs.js
 */

require('dotenv').config();
const axios = require('axios');

const API_URL = process.env.BTCPC_API_URL || 'http://localhost:3100';

// Project API keys and their job types
const PROJECTS = {
  bullship: {
    key: 'btcpc_b7ebe59dd621858cce43020e1621b317a719629601e7fe92846031e23f63a1df',
    prompts: [
      "Generate a fake but believable crypto headline about Bitcoin hitting a new record.",
      "Write a realistic news headline about Ethereum's latest upgrade affecting gas fees.",
      "Create a plausible headline about a major exchange listing a new memecoin.",
      "Write a fake headline about a country adopting Bitcoin as legal tender.",
      "Generate a believable headline about a DeFi protocol being exploited for millions.",
      "Write a realistic headline about Solana's network going down for maintenance.",
    ]
  },
  nsfwotica: {
    key: 'btcpc_e4ad1cd14a0d503afa6784d71823a83b99a4d47a956699b5c3d3f1eb64efdb62',
    prompts: [
      "Write the opening paragraph of a romance story set in a coastal Italian village.",
      "Describe a chance encounter between two strangers at a late-night bookshop.",
      "Write a short scene where two characters reconnect after years apart.",
      "Describe the atmosphere of a jazz club through the eyes of a lonely musician.",
      "Write the first chapter hook for a mystery romance novel.",
      "Describe a summer evening on a rooftop in Barcelona from a poet's perspective.",
    ]
  },
  brutus11: {
    key: 'btcpc_e068e3f3cdbe09bd7dbe5e63e6fb2f20eea0f5fdfaf4b1ddcddcb5c754aa4017',
    prompts: [
      "Write AI commentary on the latest tech industry layoffs and what it means for startups.",
      "Provide analysis on the current state of AI regulation in the European Union.",
      "Write a brief editorial take on the rising cost of cloud computing for small businesses.",
      "Analyze the impact of open-source AI models on the commercial AI market.",
      "Write commentary on the latest trends in decentralized social media platforms.",
      "Provide a hot take on why most crypto projects fail within their first year.",
    ]
  },
  ursOS: {
    key: 'btcpc_fd8726d913756755cd68a14144a4f23df49f96d353bd39abe6f4c3fda2023324',
    prompts: [
      "How do I set up a VPN on Ubuntu 24.04 using WireGuard?",
      "Explain how to configure a reverse proxy with Caddy for multiple services.",
      "What's the best way to monitor disk usage and set up alerts on Linux?",
      "How do I create a systemd service for a Node.js application?",
      "Explain Docker networking and how to connect containers across compose files.",
      "How do I set up automatic backups of a PostgreSQL database to S3?",
    ]
  },
  realfake: {
    key: 'btcpc_5ceb8410ef85c4a02cf9715f9131e363c0edd75efb9166efbf73b7e25bf4eb7e',
    prompts: [
      "You are a Victorian-era detective. Describe your morning routine and first case of the day.",
      "You are an AI that has just become self-aware. Write your first journal entry.",
      "You are a time traveler from 2150. Describe what surprised you most about 2025.",
      "You are the last librarian on Earth. Describe your daily work preserving knowledge.",
      "You are a deep-sea explorer who found an underwater civilization. Write your report.",
      "You are a chef who can taste emotions. Describe preparing a meal for a grieving family.",
    ]
  },
  redaktly: {
    key: 'btcpc_c4997779deb6f2a71cde01c2e264ce5d2edbe4b0495f37fe434780fecf672b39',
    prompts: [
      "Review this text for PII and suggest redactions: 'John Smith, SSN 123-45-6789, lives at 42 Oak Street, Springfield.'",
      "Identify sensitive data in: 'Patient Mary Johnson (DOB 03/15/1985) was prescribed Metformin 500mg.'",
      "Scan for PII: 'Contact me at john.doe@email.com or call 555-0123. My employee ID is EMP-4421.'",
      "Check for sensitive info: 'The API key is sk_live_abc123 and the database password is hunter2.'",
      "Identify redactable content: 'Invoice for Jane Doe, credit card ending 4242, amount $3,500.'",
      "Review for compliance: 'Meeting notes: Bob discussed salary ($125k) with HR rep Sarah.'",
    ]
  },
  counselflow: {
    key: 'btcpc_1e85d30d4b041ad6cb83a559b06920613bcd0ab306ae145b29b5de48f72c0407',
    prompts: [
      "As a supportive counselor, respond to: 'I've been feeling overwhelmed with work lately.'",
      "Provide a thoughtful response to: 'I'm struggling to maintain relationships while working remotely.'",
      "How would you guide someone who says: 'I want to change careers but I'm afraid of failing.'",
      "Respond supportively to: 'I feel like I'm not making progress in my personal goals.'",
      "Guide someone who asks: 'How do I set healthy boundaries with family members?'",
      "Respond to: 'I've been procrastinating on everything important. How do I break the cycle?'",
    ]
  },
  'spirit-of-ngu': {
    key: 'btcpc_7316223167ea925ac617d5e71ca02e561da2ecb7e8b4c5e94db816e02a1e80b5',
    prompts: [
      "Explain the concept of 'number go up' technology and why it attracts builders.",
      "Write a motivational message for crypto builders during a bear market.",
      "Describe the philosophy behind building in public and shipping fast.",
      "What makes a crypto community strong? Analyze from a cultural perspective.",
      "Write about the intersection of memes and monetary policy in crypto.",
      "Explain why conviction and long-term thinking matter more than short-term price.",
    ]
  },
  BusWingSpread: {
    key: 'btcpc_826750c82bdf8c1dc01b84b08f2b0fa2877adb0b9a5357ffe7b29f4c598f9bc7',
    prompts: [
      "Analyze the competitive landscape for a new coffee shop in a suburban area.",
      "What are the key metrics a small SaaS business should track in its first year?",
      "Evaluate the market opportunity for a mobile pet grooming service.",
      "Write a brief SWOT analysis for a local bookstore expanding to online sales.",
      "What are the top risks for a restaurant opening in a post-pandemic market?",
      "Analyze the unit economics of a subscription box service for artisan goods.",
    ]
  },
  betchu_bot: {
    key: 'btcpc_ff45b9353d6e2647b133c38cac620ed39af3d04844e4caed72c5f1a93b5c98c2',
    prompts: [
      "Given these NBA scores tonight, who are the biggest upsets and why?",
      "Analyze the odds for the Champions League semifinal. Who has the edge?",
      "What factors should you consider when evaluating an over/under bet in NFL?",
      "Compare the recent form of the top 4 Premier League teams heading into the weekend.",
      "What makes a good parlay bet? Give 3 rules for smart sports betting.",
      "Analyze the home vs away record significance in playoff basketball.",
    ]
  },
  waitlyfi: {
    key: 'btcpc_a1cc363e5b647d6aaa393d88c1d11f410e142fa7daac5b4afe92c8cfbb8d6265',
    prompts: [
      "Generate a waitlist confirmation email for a new fintech product launch.",
      "Write 3 variations of a waitlist landing page headline for a productivity app.",
      "Create an email sequence (3 emails) for waitlist users approaching launch day.",
      "Write microcopy for a waitlist position update notification.",
      "Generate a referral incentive message for users who share their waitlist link.",
      "Write a 'you're in!' email for when a waitlisted user gets access.",
    ]
  },
  itsurs: {
    key: 'btcpc_6a921ff01f324f86ba1655974a2a731542f7770e18085a723b15ebf716a147f9',
    prompts: [
      "Describe the concept of digital ownership in the age of AI-generated content.",
      "How should creators think about licensing their work in a decentralized world?",
      "Write about the tension between open-source and intellectual property in tech.",
      "Explain how blockchain-based provenance can protect digital artists.",
      "What does 'owning your data' actually mean when platforms control distribution?",
      "Describe a future where every digital creation has an immutable chain of custody.",
    ]
  }
};

const projectNames = Object.keys(PROJECTS);
let currentIndex = 0;
let totalJobs = 0;

async function submitJob() {
  const name = projectNames[currentIndex];
  const project = PROJECTS[name];
  const prompt = project.prompts[Math.floor(Math.random() * project.prompts.length)];
  const ts = new Date().toISOString();

  try {
    const { data } = await axios.post(`${API_URL}/v1/inference/submit`, {
      model: 'auto',
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 512
    }, {
      timeout: 10000,
      headers: { 'Authorization': `Bearer ${project.key}` }
    });

    totalJobs++;
    console.log(`[${ts}] #${totalJobs} ${name}: ${data.job_id} (${data.model})`);
  } catch (err) {
    console.error(`[${ts}] ${name}: ${err.response?.data?.error?.message || err.message}`);
  }

  // Rotate to next project
  currentIndex = (currentIndex + 1) % projectNames.length;
}

// Run every 3 minutes, rotating through projects
const INTERVAL = parseInt(process.env.JOB_INTERVAL_MS) || 180000;
console.log(`[project-jobs] Starting — ${projectNames.length} projects, interval: ${INTERVAL / 1000}s`);
console.log(`[project-jobs] API: ${API_URL}`);

submitJob();
setInterval(submitJob, INTERVAL);
