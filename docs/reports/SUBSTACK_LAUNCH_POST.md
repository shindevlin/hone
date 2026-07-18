# Substack Launch Post
# Title: The work is the point

---

**The work is the point**

Bitcoin solved a hard problem: how do you create money without a central bank?

The answer was proof of work. Make the money expensive to produce, so the cost of producing it is the guarantee of its value. The work was the proof.

The problem is that the work produces nothing else. A Bitcoin mining rig burns electricity solving puzzles that exist only to be solved. The output is the hash. The hash is the product. The hash is useless to anyone except the network that asked for it.

That's not a criticism of Bitcoin. It's a design choice that made sense for a digital gold network. For money, you want the cost to be obvious and the output to be simple.

But what if the work could be the point?

---

**What HONE is**

HONE is a blockchain where mining means doing something useful.

Every 30 seconds, the network evaluates what each connected machine actually did. A GPU that ran AI inference jobs earns from the miner pool. A Raspberry Pi that passively detected Bluetooth trackers earns from the sensor pool. A server that hosted encrypted files earns from the storage pool. A phone that kept the network's epoch timing earns from the clock pool.

No synthetic puzzles. No arbitrary hashes. The work is the product. The token is the receipt.

The supply is fixed at 42,000,000 HONE, like Bitcoin. Nothing is burned — all fees recycle back into future epoch rewards and re-emerge as earnings for whoever is doing real work.

---

**The demand problem**

Every new blockchain faces the same credibility question: why does the token have value?

The easy answer is "because miners want it to pay operating costs." That's circular. Miners earn tokens that are valuable because miners want them. You can run that logic for a while, but it's a circle.

The honest answer requires showing where demand comes from outside the mining community. For Bitcoin, the answer is: people want digital gold. The security properties and scarcity make it worth holding. For Ethereum, the answer is: gas fees for a vast application ecosystem.

For HONE, the answer is three native markets that launched with the chain.

---

**Verasens: the data market**

Your Raspberry Pi sits on your desk and does nothing most of the day.

Install the HONE node, enable BLE scanning, and it starts passively detecting AirTags, Android Find My devices, Tile trackers, and Samsung SmartTags that move through your area. No pairing. No interaction with the devices. It just listens.

Device owners who want to track their property without going through Apple or Google can pay a subscription fee in HONE to receive encrypted sighting data. When their tag passes through your Pi's coverage area, you get a cut of that fee.

Scale that up. A city with Pis on every block is a city where lost things get found, where logistics companies know where their shipments are, where the data doesn't flow through a single corporation that can sell it, lose it, or shut it off.

Verasens is the market where that data gets bought and sold. Query fees go to sensor nodes. Subscription fees split between observer nodes, storage nodes that host the encrypted sighting data, and treasury. The chain is the verification layer — the data is attested, timestamped, and can't be fabricated after the fact.

---

**Freeport: the market with no platform**

Amazon takes 15%. Etsy takes 6.5% plus payment processing. Shopify charges a monthly fee and a transaction fee. Stripe has a terms of service that can freeze your account with 90 days notice and no appeal.

Every marketplace is a toll booth. You build your store on someone else's land, pay rent in perpetuity, and accept that the rules can change after you've built a business on them.

Freeport is peer-to-peer commerce built into the chain. Buyer and seller transact directly. Payment goes into escrow when the order is placed, releases on delivery confirmation. Digital goods fulfill automatically — the seller uploads an encrypted file linked to the product listing, and it decrypts to the buyer's key the moment payment clears. Zero seller action required.

There's no company in the middle because there's no company. There's only the ledger. Storage nodes earn from listing fees. Service nodes earn from API traffic. The marketplace exists as long as the chain exists.

---

**LinkGit: where code lives permanently**

GitHub was acquired by Microsoft in 2018. That was fine, mostly. But it won't always be fine, and the track record of large acquisitions preserving developer tools indefinitely is not encouraging.

More immediately: GitHub has a terms of service. Projects get removed. Accounts get suspended. Repositories that existed yesterday sometimes don't exist tomorrow.

LinkGit is decentralized, Git-compatible code hosting on HONE's distributed filesystem. You push and pull with standard Git tooling. The objects — commits, trees, blobs — are stored content-addressed across storage nodes on the network. Private repos are encrypted to your key at rest; no storage node can read the contents. Dead objects prune automatically; owners pay to retain them beyond the default window.

There's no terms of service that changes after you've published. The repo lives as long as someone is paying storage fees and storage nodes are serving it. Which, given the economics of the storage pool, is indefinitely.

---

**The machine you already own**

The thing I want to be direct about: you don't need to buy anything to participate.

The phone charging on your nightstand can run the clock and sensor roles in the browser. The Raspberry Pi sitting on your shelf earning dust is a full-featured sensor node in twenty minutes. The gaming GPU that idles at night can run Ollama and fulfill AI inference jobs while you sleep.

The network pays for useful work. You probably have hardware that can do useful work. The gap between those two facts is a twenty-minute install.

If you have a GPU, install Ollama, pull a model, and point the node at your account. You'll see the first epoch reward in thirty seconds.

If you have a Pi, enable BLE scanning and watch it start detecting trackers in your neighborhood. When those trackers' owners subscribe to sighting data, you start earning from their subscription.

If you have nothing but a browser, open honemesh.net/clock and run the browser clock. The clock pool pays for keeping the network's epoch timing alive. Any device qualifies.

---

**Why now**

The AI compute market is being built right now. The question is who owns the machines underneath it.

Right now, almost all AI inference routes through AWS, Azure, or Google Cloud. The three companies. The same three companies that own the search engines, the cloud storage, the marketplaces, the code repositories, and the advertising infrastructure.

HONE is a bet that those things can run on machines that individuals own. That the GPU in your gaming rig can answer real inference jobs. That the Pi on your desk can report real sensor data. That the storage drive you're not using can host files that people actually need.

It's not a bet on a future state. The chain is live. The work is happening. The rewards are flowing.

Pick your hardware. Start earning.

---

*honemesh.net — Telegram: @honebot — GitHub: shindevlin/hone*
