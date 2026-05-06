# Reddit Launch Post
# Target: r/btcpc, cross-post to r/selfhosted, r/homelab, r/raspberry_pi, r/MachineLearning

---

**Title:**
BTCPC is live — mine with your GPU, Raspberry Pi, or phone (the work is actual AI inference, not pointless hashes)

---

**Body:**

I built a blockchain where mining means doing something useful.

**What it actually is:**

Every 30 seconds, the network looks at what each machine did that epoch and pays proportionally:

- GPU running Ollama → fulfills real AI inference jobs someone submitted → earns from the miner pool
- Raspberry Pi → passively scans for AirTags, Android Find My, Tile trackers (no pairing) → earns from the tracker coverage pool when owners subscribe
- Phone → enables sensors and clock → earns from the sensor and clock pools
- Spare SSD → hosts encrypted files → earns from the storage pool
- Laptop → runs the browser clock → earns from the clock pool, no install needed

Nothing is burned. All fees recycle back into future epoch rewards. 42,000,000 BTCPC total supply, fixed.

**Where the demand comes from (this is the important part):**

Most crypto token pitches are circular — miners earn tokens that are valuable because miners want them. That's a circle.

BTCPC has three native markets that create real demand:

- **Verasens** — sensor data marketplace. If you run a Pi that reports BLE tracker sightings, telecoms and logistics companies can pay BTCPC to query that data. You earn from query fees.
- **Freeport** — peer-to-peer commerce without Amazon or Stripe in the middle. Sellers pay storage and settlement fees. Storage nodes earn.
- **LinkGit** — decentralized Git. Push your repos to BTCPC-FS instead of GitHub. Developers pay storage per object. Permanent, encrypted, no terms of service that can change.

These three are why the token has a reason to exist beyond speculation.

**Hardware that works right now:**

| Device | Role | Setup time |
|--------|------|-----------|
| Any phone | Clock + sensors | 2 min, in browser |
| Laptop (no GPU) | Clock node | 1 min, in browser |
| GPU rig | AI miner + clock | ~15 min |
| Raspberry Pi | BLE tracker + sensor + clock | ~20 min |
| Spare drive | Storage host | ~10 min |

**Quick start:**

```bash
# GPU mining (install Ollama first)
curl -fsSL https://btcpc.net/install.sh | sudo bash
BTCPC_ACCOUNT=yourname btcpc-node

# Or just open the browser clock right now
# btcpc.net/clock
```

Telegram wallet (no install): message @btcpcbot `/create yourname`

Android APK direct download on the site.

Happy to answer questions about the architecture. The Rust node source is on GitHub.

---

**Comments to prep:**

Q: How is this different from Helium?
A: Helium pays for LoRa gateway coverage. BTCPC pays for a broader set of work — AI inference, BLE tracker coverage (passive, no LoRa required), storage, service hosting, and clock timing. The three native markets (Verasens, Freeport, LinkGit) mean the token has buyers beyond just other node operators. Also no Helium-style governance drama — no separate hotspot token, no company in the middle.

Q: What prevents the miner pool from being gamed?
A: Inference results are verified by randomly selected verifier nodes using the same model and input. Verifiers submit approved/rejected verdicts. Only approved work earns. Unverified or rejected outputs earn nothing.

Q: Is this proof of stake?
A: No. Stake increases your reward weight but you can participate with zero stake. The work is the primary requirement — stake is a multiplier, not a gate.

Q: What model do I need?
A: Any Ollama model. Bigger models earn more because parameter count is verified from Ollama's `/api/show` metadata and used in the reward calculation. You can start with qwen2.5:0.5b on a laptop CPU.
