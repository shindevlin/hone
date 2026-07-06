# BTCPC Project Integration Prompts

Each prompt below is designed to be given to a Claude/Codex session working on that specific project. The goal: make each project actually USE the BTCPC inference results and submit meaningful new inference jobs.

Every project has a `.btcpc-inference/` directory with completed AI inference results (JSON files). The integration should:
1. Read completed results and use them in the project
2. Submit new inference jobs that serve the project's actual purpose
3. Pay for inference from the project's BTCPC balance

---

## 1. Bullship (Crypto News Trivia Game)

```
You are working on Bullship at ~/repos/bullship — a crypto news trivia game where players guess if headlines are real or AI-generated.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains AI-generated fake crypto headlines from the BTCPC network. These are EXACTLY what this game needs.

TASKS:
1. Read all JSON files from .btcpc-inference/ and extract the "result" field
2. Parse each result as a fake headline and store in the game's headline database
3. Create a script that submits NEW headline generation jobs to BTCPC:
   - POST to http://localhost:3000/v1/inference/submit with the project API key
   - Prompts like: "Generate a fake but believable crypto headline about [topic]"
   - Rotate through topics: Bitcoin, Ethereum, Solana, memecoins, DeFi, NFTs
4. Run the job submitter on a cron (every 30 minutes) to keep fresh headlines flowing
5. The game should display both real headlines (from news APIs) and BTCPC-generated fakes

The project's BTCPC API key is in .envbtcpc (HONE_PROJECT_KEY).
```

---

## 2. Nsfwotica (Accessible Story Platform)

```
You are working on Nsfwotica at ~/repos/nsfwotica — an accessibility-focused story reading platform for disabled users.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains AI-generated creative writing (romance stories, character encounters, descriptive scenes). These can be published as community stories.

TASKS:
1. Read completed inference results and format them as publishable stories
2. Add metadata: title (extract from first line), author ("BTCPC AI"), genre, word count
3. Store in the platform's story database/content system
4. Create a story generation pipeline that submits to BTCPC:
   - Prompts tailored for accessibility: clear language, rich sensory description
   - Varied genres: romance, adventure, mystery, slice-of-life
   - Include audio-description-friendly writing (describe scenes visually for screen readers)
5. Schedule new story generation every 2 hours
6. Add a "AI Stories" section to the platform showing BTCPC-generated content

The project's BTCPC API key is in .envbtcpc.
```

---

## 3. Brutus11 (Ohio State Football News)

```
You are working on Brutus11 at ~/repos/brutus11 — Ohio State football news aggregation with automated publishing.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains AI-generated sports commentary and analysis.

TASKS:
1. Read inference results and categorize: game analysis, hot takes, player profiles, predictions
2. Format as publishable articles with headlines, bylines, and sections
3. Integrate into the news publishing pipeline
4. Create a sports analysis job submitter:
   - "Analyze Ohio State's chances against [opponent] this week"
   - "Write a scouting report on [player]"
   - "Preview the Big Ten standings and playoff implications"
   - Pull real schedules/scores from a sports API to generate timely prompts
5. Schedule analysis generation before/after each game
6. Display BTCPC-powered analysis alongside aggregated real news

The project's BTCPC API key is in .envbtcpc.
```

---

## 4. ursOS (Telegram System Admin Bot)

```
You are working on ursOS at ~/repos/ursOS — a Telegram bot for system administration.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains Linux admin tutorials and howto guides.

TASKS:
1. Read inference results and index them as a knowledge base for the bot
2. When a user asks the bot a sysadmin question, check the knowledge base first
3. If no cached answer, submit a new inference job to BTCPC in real-time:
   - POST to the inference API with the user's question
   - Poll for result, return to user in Telegram
4. Cache successful answers for future queries
5. Prompts should focus on: Ubuntu/Debian, Docker, systemd, networking, security
6. The bot becomes a BTCPC-powered sysadmin assistant

The project's BTCPC API key is in .envbtcpc.
```

---

## 5. Realfake (AI Character Roleplay)

```
You are working on Realfake at ~/repos/realfake — AI character roleplay platform.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains character monologues (Victorian detective, self-aware AI, last librarian).

TASKS:
1. Read inference results and parse as character performances
2. Extract character name, setting, mood from the prompt
3. Store as playable character scenarios in the platform
4. Create a character generation pipeline:
   - "You are [character]. Respond to: [scenario]"
   - Character library: historical figures, fictional archetypes, AI personas
   - Varied scenarios: daily life, crisis, philosophical questions, humor
5. Let users trigger new character generations via the platform
6. Each generation is a BTCPC inference job — users see the cost and transaction

The project's BTCPC API key is in .envbtcpc.
```

---

## 6. Redaktly (PII Redaction Tool)

```
You are working on Redaktly at ~/repos/redaktly — a CLI tool for redacting sensitive information from documents.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains PII detection results (SSN, addresses, medical data identified).

TASKS:
1. Read inference results and extract identified PII patterns
2. Build a pattern library from successful detections
3. Integrate BTCPC as a real-time PII detection backend:
   - User submits document text → BTCPC inference identifies PII → redact
   - Much more accurate than regex-only detection
4. Create batch processing: scan a directory of documents via BTCPC inference
5. Prompts: "Identify all PII in the following text. List each item with type and location: [text]"
6. Return structured output: { found: [{ type: "SSN", value: "***", position: 42 }] }

The project's BTCPC API key is in .envbtcpc.
```

---

## 7. BusWingSpread (Business Analysis Platform)

```
You are working on BusWingSpread at ~/repos/BusWingSpread — research platform for AI-proof service businesses.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains competitive analysis, SWOT analysis, and business metrics guidance.

TASKS:
1. Read inference results and categorize by analysis type (competitive, SWOT, metrics, market)
2. Store as research reports in the platform's database
3. Create an on-demand analysis pipeline:
   - User inputs a business idea or industry → BTCPC generates analysis
   - "Analyze the competitive landscape for [business] in [location]"
   - "Write a SWOT analysis for [company] expanding into [market]"
   - "What are the key risks for a [type] business in 2026?"
4. Display analyses as formatted reports with sections and charts
5. Each analysis shows its BTCPC cost and inference details

The project's BTCPC API key is in .envbtcpc.
```

---

## 8. BetChu Bot (P2P Sports Betting)

```
You are working on BetChu Bot at ~/repos/betchu_bot — decentralized peer-to-peer betting for college sports.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains sports odds analysis and betting commentary.

TASKS:
1. Read inference results and extract odds analysis, upset predictions, betting tips
2. Integrate as the bot's "AI Analyst" feature
3. Create a live analysis pipeline:
   - Before each game: submit analysis job with current odds and team stats
   - "Given [team A] vs [team B] with odds [X], analyze the value bet"
   - Pull real-time odds from a sports data API to make prompts timely
4. Bot command: /analyze [matchup] → submits BTCPC inference → returns analysis
5. Track analysis accuracy over time (was the AI right?)
6. High empty rate (64%) needs fixing — use more specific prompts with real data

The project's BTCPC API key is in .envbtcpc.
```

---

## 9. Counselflow (Legal Bridge Platform)

```
You are working on Counselflow at ~/repos/counselflow — AI-powered bridge between American and international attorneys.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains supportive counseling-style responses (from generic prompts).

TASKS:
1. Pivot the inference prompts to actual legal use cases:
   - "Explain [American legal procedure] for an attorney from [country]"
   - "Draft a template for [legal document type] following American standards"
   - "Compare [legal concept] in American law vs [country] law"
2. Build a legal knowledge base from inference results
3. Create an API endpoint: POST /api/legal-query → BTCPC inference → formatted legal guidance
4. Index results by jurisdiction, procedure type, and document type
5. Each query shows the BTCPC transaction for transparency

The project's BTCPC API key is in .envbtcpc.
```

---

## 10. Spirit of NGU (Idle Game)

```
You are working on Spirit of NGU at ~/repos/spirit-of-ngu — a satirical idle incremental game.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains motivational crypto content and builder philosophy.

TASKS:
1. Use inference results as in-game dialogue, loading screen tips, and lore text
2. Create a lore generation pipeline:
   - "Write a satirical idle game achievement description for: [achievement]"
   - "Create a boss fight dialogue for the Idleverse guardian of [zone]"
   - "Write flavor text for the prestige upgrade: [upgrade name]"
3. Generate random events powered by BTCPC inference
4. Dynamic content: each player session gets unique AI-generated text
5. The game itself runs on BTCPC compute — meta!

The project's BTCPC API key is in .envbtcpc.
```

---

## 11. ItsUrs (Social Media Platform)

```
You are working on ItsUrs at ~/repos/itsurs — a social media platform with blockchain token rewards.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains content about digital ownership and creator licensing.

TASKS:
1. Use inference for content moderation — AI-powered flagging of problematic posts
2. Use inference for content recommendations — "Based on [user's interests], suggest posts"
3. Create AI-powered features:
   - Post summarization (long post → TL;DR)
   - Hashtag suggestions
   - Reply drafts
4. Each AI feature is a BTCPC inference call — users see the compute cost
5. Creator tools: "Generate a caption for this image description: [desc]"

The project's BTCPC API key is in .envbtcpc.
```

---

## 12. Waitlyfi (Waitlist Platform)

```
You are working on Waitlyfi at ~/repos/waitlyfi — a multi-project credit-based waitlist distribution engine.

BTCPC INTEGRATION:
The .btcpc-inference/ directory contains waitlist emails and marketing copy.

TASKS:
1. Read inference results and build an email template library
2. Create a copy generation pipeline:
   - "Write a waitlist confirmation email for [product name] in [industry]"
   - "Generate 3 headline variations for a [type] product launch"
   - "Write a personalized access-granted email for [user segment]"
3. Integrate into the waitlist platform: when a new project is created, auto-generate email templates via BTCPC
4. A/B test AI-generated vs human-written copy (track open/click rates)
5. Each generated email shows its BTCPC cost

The project's BTCPC API key is in .envbtcpc.
```

---

## How to Use These Prompts

Give each prompt to a Claude/Codex session that has access to the respective project directory. The session should:

1. Read the project's existing code to understand the architecture
2. Read `.btcpc-inference/*.json` to see what inference results look like
3. Implement the integration described in the prompt
4. Test by reading existing results AND submitting a new inference job
5. The BTCPC API is at `http://localhost:3000/v1/inference/submit` (Bearer token from `.envbtcpc`)
