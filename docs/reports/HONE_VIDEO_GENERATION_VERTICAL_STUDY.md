# HONE Video Generation Vertical Study

Date: 2026-07-05

## Thesis

Video generation is a strong fourth HONE vertical because it turns distributed compute into an obvious commercial product: users pay HONE for finished media, while the network handles planning, model routing, generation, stitching, storage, verification, and settlement.

The differentiator should not be "type a prompt and get a clip." The differentiator should be productized outputs:

- Simple commercial
- Long-form commercial
- TikTok / short vertical video
- YouTube video
- 30-minute TV-style show
- Full cinematic movie

HONE should hide the production pipeline from normal users. The user chooses an output type, describes the product/story/channel, sets budget and style, and the system turns that into a structured production job.

## Why This Fits HONE

HONE is opt-in by design. Video generation does not need to be mandatory for every node. Workers can advertise media capabilities the same way they advertise other compute roles.

This creates a real reason for HONE to exist:

- GPU node operators get high-value work.
- Buyers get finished commercial media instead of raw inference.
- Storage nodes serve generated artifacts and project files.
- Reviewers/verifiers can participate in delivery checks and optional quality review.
- The token has clear demand: media jobs, revisions, storage, rendering, review, distribution.

## Product Shape

The user should start from intent, not from model settings.

HONE should be an A-to-Z video production tool, not a prompt box. The buyer should be able to arrive with a rough goal and leave with a finished deliverable: creative brief, script, storyboard, generated assets, voice, music, edit, captions, revisions, final render, project bundle, storage, and settlement.

The interface should avoid exposing "how the sausage is made" unless the user asks for advanced control. Normal users should choose an outcome, answer a few practical questions, approve the plan, and track progress.

Primary creation modes:

1. Simple commercial
   - 15-60 seconds
   - Product/service pitch
   - Script, voiceover, visuals, subtitles, music, call-to-action

2. Long-form commercial
   - 2-10 minutes
   - Explainer, launch video, Kickstarter-style pitch, sales page video
   - More scene planning and narrative structure

3. TikTok / short vertical video
   - 9:16 output
   - Hook-first scripting, fast captions, trend-aware pacing
   - Optional generated presenter, product shots, B-roll

4. YouTube video
   - 5-20 minutes
   - Title/thumbnail concepts, chapter outline, voiceover, visuals, intro/outro
   - Optional documentary, tutorial, review, or essay formats

5. 30-minute TV-style show
   - Episode structure
   - Cold open, acts, scenes, dialogue, continuity, credits
   - Can be animated, cinematic, documentary, news, interview, or educational

6. Full cinematic movie
   - Feature-length project
   - Multi-stage production: treatment, script, storyboard, shot list, scene generation, edit, audio, grading
   - Should be milestone-based, not a single job

Additional supported types can include music videos, trailers, course modules, real estate videos, game cutscenes, product demos, and social ad variants.

## The Core Abstraction: Video Product Presets

Each video type should be a preset that expands into a production plan.

Example fields:

- `type`: `simple_commercial`, `youtube`, `movie`, etc.
- `duration_target`
- `aspect_ratio`
- `scene_count`
- `script_required`
- `voiceover_required`
- `music_required`
- `subtitles_required`
- `revision_policy`
- `artifact_policy`
- `estimated_cost_hunits`
- `required_capabilities`

The user sees a simple form. The network sees a structured job graph.

## A-To-Z User Experience

The strongest product is a guided producer flow:

1. Goal intake
   - "What are we making?"
   - "Who is it for?"
   - "Where will it be published?"
   - "What action should viewers take?"
   - "What assets already exist?"

2. Automatic creative brief
   - HONE turns messy intent into a clear brief.
   - Claude or another LLM can ask only the missing questions.
   - The brief becomes the contract for workers and reviewers.

3. One-click production plan
   - Script beats
   - Scene list
   - Shot list
   - Voice/music/subtitle requirements
   - Editing style
   - Milestones
   - Cost and delivery options

4. Marketplace execution
   - HONE routes each step to the best available opt-in worker or local model.
   - A single video can combine local generation, remote GPU workers, human editors, voice workers, storage nodes, and reviewers.

5. Review and revision
   - The buyer reviews previews at natural checkpoints.
   - Revision requests are tied to the approved brief and scene ids.
   - Budget and deadline impact are visible before approval.

6. Final delivery
   - Final render
   - Social/channel-specific exports
   - Captions and transcript
   - Thumbnail concepts
   - Project/source bundle when available
   - Provenance and artifact CIDs

This makes HONE feel like a production studio that can scale down to a $20 local ad or up to a long-form cinematic project.

## Unique Marketplace Angle

The marketplace differentiator is not simply that HONE can generate video. Many products will do that.

HONE can be different because it can coordinate a complete production supply chain:

- AI models for script, image, video, voice, music, captions, and editing
- Local user-owned models for private or low-cost generation
- Distributed GPU workers for scale and speed
- Human specialists for edits, polish, compliance, localization, and review
- Storage nodes for durable artifact hosting
- Reputation, escrow, milestones, disputes, and revision economics

This allows HONE to sell a finished outcome instead of selling credits for a single model. The buyer does not need to know whether the best path uses one local model, five network workers, a human editor, or a hybrid. HONE can choose the path based on budget, deadline, privacy, quality, and available capabilities.

## Pipeline

1. Intake
   - User selects video product type.
   - User provides goal, topic/product, audience, tone, references, constraints, and budget.

2. Planning
   - HONE generates a treatment, outline, script, shot list, and production plan.
   - Long projects are split into milestones.

3. Worker Matching
   - Nodes advertise capabilities:
     - `video_generation`
     - `image_generation`
     - `voice_synthesis`
     - `music_generation`
     - `editing`
     - `upscaling`
     - `subtitle_rendering`
     - `vram_gb`
     - `max_resolution`
     - `max_duration_seconds`
     - supported models/tools

4. Generation
   - Workers generate clips, stills, voiceover, music, captions, and transitions.
   - Large projects run as batches of scene jobs.

5. Assembly
   - Editing worker stitches clips, audio, captions, and metadata.
   - Outputs preview and final render CIDs.

6. Review
   - Buyer accepts, requests revisions, or opens a dispute.
   - Optional reviewer marketplace checks delivery against the job spec.

7. Settlement
   - Escrow releases by milestone or final acceptance.
   - Storage fees apply for artifact retention.

## Protocol Model

Start as an opt-in marketplace capability before making it consensus-critical.

Potential ledger entries:

- `MediaJobPost`
- `MediaJobClaim`
- `MediaJobPlan`
- `MediaAssetComplete`
- `MediaJobAssemble`
- `MediaJobReview`
- `MediaJobAccept`
- `MediaJobDispute`
- `MediaJobPay`

Avoid subjective quality in consensus. Consensus should verify:

- job terms
- signatures
- escrow
- worker claim
- artifact CIDs
- hashes
- timestamps
- delivery windows
- revision/dispute windows

Subjective quality should live in buyer acceptance, reviewer markets, reputation, and dispute outcomes.

## Pricing Model

Price by product type and complexity:

- duration
- resolution
- aspect ratio
- frame rate
- number of scenes
- model class
- voice/music/subtitles
- revision count
- storage duration
- rush delivery
- worker reputation

Full movies and 30-minute shows should use milestone escrow:

- concept/treatment
- script
- storyboard/shot list
- scene batch 1
- scene batch 2
- assembly
- final render

## Safety And Policy

Because all nodes are opt-in, HONE should support node-level policy controls.

Default public marketplace policy should restrict:

- non-consensual deepfakes
- sexual content involving real people or minors
- impersonation for fraud
- explicit political deception
- copyrighted character/style abuse where legally risky

Node operators can choose stricter local policies. Job metadata should expose policy requirements so workers can opt in or out before claiming.

## MVP Recommendation

Build the first version around three product presets:

1. Simple commercial
2. TikTok / short vertical video
3. YouTube video

These prove the workflow without committing to long-form orchestration immediately.

Then add:

4. Long-form commercial
5. 30-minute TV-style show
6. Full cinematic movie

The long-form formats should be milestone-based from day one.

## Differentiator

Most tools sell model access. HONE should sell finished media workflows.

The user should not need to know:

- which model generated which clip
- which worker rendered which scene
- how voice/music/subtitles were assembled
- where artifacts are stored
- how escrow settles

The user should choose the kind of finished video they want, approve the plan, and receive production-ready output.
