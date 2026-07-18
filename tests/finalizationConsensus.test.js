describe('finalizationConsensus', () => {
  // Loads a fresh module with side effects neutralized: no ledger writes,
  // no stake (count-based voting), no persisted chain state. Individual
  // tests override providers to exercise stake/persistence paths.
  function loadConsensus(overrides = {}) {
    jest.resetModules();
    const consensus = require('../src/chain/finalizationConsensus');
    consensus._setPersistProvider(overrides.persist || (() => {}));
    consensus._setStakeProvider(overrides.stake || (() => 0));
    consensus._setProofProvider(overrides.proofs || (() => ({ miningProofs: [], computeProofs: [] })));
    return consensus;
  }

  beforeEach(() => {
    jest.useRealTimers();
    delete process.env.HONE_PROPOSAL_WINDOW_MS;
    delete process.env.HONE_PROPOSAL_TIMEOUT_MS;
    delete process.env.HONE_MIN_CONSENSUS_PROPOSALS;
    delete process.env.HONE_MIN_CONSENSUS_PEERS;
    delete process.env.HONE_MAX_PROPOSAL_TOTAL;
    delete process.env.HONE_STRICT_PROPOSAL_VALIDATION;
    const stateStore = require('../src/chain/stateStore');
    stateStore.resetAll();
  });

  test('two proposals with same rewards produce the same hash and reach majority consensus', () => {
    const consensus = loadConsensus();
    const rewardsA = [
      { miner: 'shindevlin', amount: 100 },
      { miner: 'natoshisakamoto', amount: 143.0555555556 }
    ];
    const rewardsB = [
      { miner: 'natoshisakamoto', amount: 143.0555555556 },
      { miner: 'shindevlin', amount: 100 }
    ];

    const hashA = consensus.hashRewards(rewardsA, 1000, 2);
    const hashB = consensus.hashRewards(rewardsB, 1000, 2);
    expect(hashA).toBe(hashB);

    const first = consensus.submitProposal(100, {
      proposer: 'shindevlin',
      rewards: rewardsA,
      total_work: 1000,
      settled_jobs: 2,
      timestamp: 100
    });
    const second = consensus.submitProposal(100, {
      proposer: 'natoshisakamoto',
      rewards: rewardsB,
      total_work: 1000,
      settled_jobs: 2,
      timestamp: 200
    });

    expect(first.accepted).toBe(true);
    expect(first.consensus).toBe(false);
    expect(second.accepted).toBe(true);
    expect(second.consensus).toBe(true);
    expect(consensus.isResolved(100)).toBe(true);
    expect(consensus.getWinner(100).proposer).toBe('shindevlin');
  });

  test('different reward groups tie on count and highest work wins', () => {
    const consensus = loadConsensus();

    consensus.submitProposal(101, {
      proposer: 'miner-a',
      rewards: [{ miner: 'miner-a', amount: 120 }],
      total_work: 50,
      settled_jobs: 1,
      timestamp: 200
    });

    consensus.submitProposal(101, {
      proposer: 'miner-b',
      rewards: [{ miner: 'miner-b', amount: 121 }],
      total_work: 75,
      settled_jobs: 1,
      timestamp: 100
    });

    const winner = consensus.resolve(101);
    expect(winner.proposer).toBe('miner-b');
    expect(consensus.amIBroadcaster(101, 'miner-b')).toBe(true);
    expect(consensus.amIBroadcaster(101, 'miner-a')).toBe(false);
  });

  test('single proposal wins after timeout window expires', () => {
    jest.useFakeTimers();
    process.env.HONE_PROPOSAL_WINDOW_MS = '100';
    const consensus = loadConsensus();

    const result = consensus.submitProposal(102, {
      proposer: 'solo-miner',
      rewards: [{ miner: 'solo-miner', amount: 243.0555555556 }],
      total_work: 100,
      settled_jobs: 1,
      timestamp: 100
    });

    expect(result.consensus).toBe(false);
    jest.advanceTimersByTime(100);

    expect(consensus.isResolved(102)).toBe(true);
    expect(consensus.getWinner(102).proposer).toBe('solo-miner');
  });

  test('duplicate proposer is rejected', () => {
    const consensus = loadConsensus();

    const first = consensus.submitProposal(103, {
      proposer: 'shindevlin',
      rewards: [{ miner: 'shindevlin', amount: 100 }],
      total_work: 100,
      settled_jobs: 1,
      timestamp: 100
    });
    const second = consensus.submitProposal(103, {
      proposer: 'shindevlin',
      rewards: [{ miner: 'shindevlin', amount: 100 }],
      total_work: 100,
      settled_jobs: 1,
      timestamp: 101
    });

    expect(first.accepted).toBe(true);
    expect(second.accepted).toBe(false);
    expect(consensus.getProposals(103)).toHaveLength(1);
    consensus.resolve(103);
  });

  test('majority reward hash wins over a conflicting higher-work minority', () => {
    const consensus = loadConsensus();
    const majorityRewards = [{ miner: 'alice', amount: 50 }];
    const minorityRewards = [{ miner: 'mallory', amount: 499 }];

    consensus.submitProposal(105, {
      proposer: 'miner-a',
      rewards: majorityRewards,
      total_work: 10,
      settled_jobs: 1,
      timestamp: 100
    });

    consensus.submitProposal(105, {
      proposer: 'miner-b',
      rewards: minorityRewards,
      total_work: 10000,
      settled_jobs: 1,
      timestamp: 101
    });

    const result = consensus.submitProposal(105, {
      proposer: 'miner-c',
      rewards: majorityRewards,
      total_work: 10,
      settled_jobs: 1,
      timestamp: 102
    });

    expect(result.consensus).toBe(true);
    expect(consensus.getWinner(105).proposer).toBe('miner-a');
    expect(consensus.getWinner(105).consensus_hash).toBe(consensus.hashRewards(majorityRewards, 10, 1, 105));
  });

  test('hashRewards is order-independent by miner name', () => {
    const consensus = loadConsensus();
    const hashA = consensus.hashRewards([
      { miner: 'zeta', amount: 10.12345678912 },
      { miner: 'alpha', amount: 4.5 }
    ]);
    const hashB = consensus.hashRewards([
      { miner: 'alpha', amount: 4.5 },
      { miner: 'zeta', amount: 10.12345678912 }
    ]);

    expect(hashA).toBe(hashB);
  });

  test('amIBroadcaster is true only for the earliest proposer in the winning group', () => {
    const consensus = loadConsensus();
    const rewards = [{ miner: 'shared', amount: 200 }];

    consensus.submitProposal(104, {
      proposer: 'late-miner',
      rewards,
      total_work: 100,
      settled_jobs: 1,
      timestamp: 200
    });

    consensus.submitProposal(104, {
      proposer: 'early-miner',
      rewards,
      total_work: 100,
      settled_jobs: 1,
      timestamp: 100
    });

    expect(consensus.isResolved(104)).toBe(true);
    expect(consensus.getWinner(104).proposer).toBe('early-miner');
    expect(consensus.amIBroadcaster(104, 'early-miner')).toBe(true);
    expect(consensus.amIBroadcaster(104, 'late-miner')).toBe(false);
  });

  // ── Stake-weighted voting ──────────────────────────────────────

  test('high-stake minority outvotes a zero-stake Sybil swarm', () => {
    const stakes = { whale: 10000, sybil1: 0, sybil2: 0, sybil3: 0 };
    const consensus = loadConsensus({ stake: (p) => stakes[p] || 0 });
    const honestRewards = [{ miner: 'whale', amount: 243 }];
    const sybilRewards = [{ miner: 'mallory', amount: 243 }];

    consensus.submitProposal(200, { proposer: 'whale', rewards: honestRewards, total_work: 100, settled_jobs: 1, timestamp: 1 });
    const result = consensus.submitProposal(200, { proposer: 'sybil1', rewards: sybilRewards, total_work: 1, settled_jobs: 1, timestamp: 2 });

    // Whale holds 100% of submitted stake → stake majority despite the disagreeing Sybil
    expect(result.consensus).toBe(true);
    expect(consensus.getWinner(200).proposer).toBe('whale');
    expect(consensus.getWinner(200).consensus_stake).toBe(10000);
  });

  test('stake-split below majority does not resolve early', () => {
    const stakes = { a: 500, b: 500 };
    const consensus = loadConsensus({ stake: (p) => stakes[p] || 0 });

    consensus.submitProposal(201, { proposer: 'a', rewards: [{ miner: 'a', amount: 10 }], total_work: 1, settled_jobs: 1, timestamp: 1 });
    const result = consensus.submitProposal(201, { proposer: 'b', rewards: [{ miner: 'b', amount: 20 }], total_work: 1, settled_jobs: 1, timestamp: 2 });

    // 500 vs 500 — neither group exceeds half of total stake (1000)
    expect(result.consensus).toBe(false);
    expect(consensus.isResolved(201)).toBe(false);
    consensus.resolve(201);
  });

  test('zero stake everywhere falls back to count-based majority', () => {
    const consensus = loadConsensus({ stake: () => 0 });
    const rewards = [{ miner: 'x', amount: 5 }];

    consensus.submitProposal(202, { proposer: 'a', rewards, total_work: 1, settled_jobs: 1, timestamp: 1 });
    const result = consensus.submitProposal(202, { proposer: 'b', rewards, total_work: 1, settled_jobs: 1, timestamp: 2 });

    expect(result.consensus).toBe(true);
    expect(consensus.getWinner(202).proposer).toBe('a');
  });

  test('resolve picks the highest-stake group even when outnumbered', () => {
    // Min-proposal floor of 4 keeps the swarm from auto-resolving before the whale votes
    process.env.HONE_MIN_CONSENSUS_PROPOSALS = '4';
    const stakes = { whale: 900, s1: 10, s2: 10, s3: 10 };
    const consensus = loadConsensus({ stake: (p) => stakes[p] || 0 });
    const whaleRewards = [{ miner: 'whale', amount: 100 }];
    const swarmRewards = [{ miner: 'mallory', amount: 100 }];

    consensus.submitProposal(203, { proposer: 's1', rewards: swarmRewards, total_work: 1, settled_jobs: 1, timestamp: 1 });
    consensus.submitProposal(203, { proposer: 's2', rewards: swarmRewards, total_work: 1, settled_jobs: 1, timestamp: 2 });
    consensus.submitProposal(203, { proposer: 's3', rewards: swarmRewards, total_work: 1, settled_jobs: 1, timestamp: 3 });
    consensus.submitProposal(203, { proposer: 'whale', rewards: whaleRewards, total_work: 1, settled_jobs: 1, timestamp: 4 });

    const winner = consensus.resolve(203);
    expect(winner.proposer).toBe('whale');
  });

  // ── HONE_MIN_CONSENSUS_PROPOSALS ──────────────────────────────

  test('window expiry does not auto-resolve below the minimum proposal count', () => {
    jest.useFakeTimers();
    process.env.HONE_PROPOSAL_WINDOW_MS = '100';
    process.env.HONE_MIN_CONSENSUS_PROPOSALS = '2';
    const consensus = loadConsensus();
    const rewards = [{ miner: 'solo', amount: 243 }];

    consensus.submitProposal(210, { proposer: 'solo', rewards, total_work: 1, settled_jobs: 1, timestamp: 1 });
    jest.advanceTimersByTime(100);
    expect(consensus.isResolved(210)).toBe(false);

    // Second proposal arrives — the 10s recheck interval resolves it
    consensus.submitProposal(210, { proposer: 'peer', rewards, total_work: 1, settled_jobs: 1, timestamp: 2 });
    jest.advanceTimersByTime(10000);
    expect(consensus.isResolved(210)).toBe(true);
  });

  // ── Proposal validation ────────────────────────────────────────

  test('proposal with negative or non-finite reward amounts is rejected', () => {
    const consensus = loadConsensus();
    const bad = consensus.submitProposal(220, {
      proposer: 'evil', rewards: [{ miner: 'evil', amount: -5 }], total_work: 1, settled_jobs: 1, timestamp: 1
    });
    expect(bad.accepted).toBe(false);
    expect(bad.reason).toMatch(/invalid reward amount/);

    const nan = consensus.submitProposal(220, {
      proposer: 'evil2', rewards: [{ miner: 'evil2', amount: NaN }], total_work: 1, settled_jobs: 1, timestamp: 1
    });
    expect(nan.accepted).toBe(false);
    expect(consensus.getProposals(220)).toHaveLength(0);
  });

  test('proposal exceeding the total reward cap is rejected', () => {
    const consensus = loadConsensus();
    const jackpot = consensus.submitProposal(221, {
      proposer: 'mallory', rewards: [{ miner: 'mallory', amount: 999999 }], total_work: 1, settled_jobs: 1, timestamp: 1
    });
    expect(jackpot.accepted).toBe(false);
    expect(jackpot.reason).toMatch(/exceed cap/);
  });

  test('strict mode rejects reward recipients with no recorded proof', () => {
    process.env.HONE_STRICT_PROPOSAL_VALIDATION = '1';
    const consensus = loadConsensus({
      proofs: () => ({ miningProofs: [{ miner: 'honest' }], computeProofs: [] })
    });

    const bad = consensus.submitProposal(222, {
      proposer: 'mallory', rewards: [{ miner: 'ghost', amount: 10 }], total_work: 1, settled_jobs: 1, timestamp: 1
    });
    expect(bad.accepted).toBe(false);
    expect(bad.reason).toMatch(/no recorded proof/);

    const good = consensus.submitProposal(222, {
      proposer: 'honest', rewards: [{ miner: 'honest', amount: 10 }, { miner: 'hone_recycle', amount: 5 }],
      total_work: 1, settled_jobs: 1, timestamp: 2
    });
    expect(good.accepted).toBe(true);
  });

  test('strict mode is permissive when no local proofs exist for the epoch', () => {
    process.env.HONE_STRICT_PROPOSAL_VALIDATION = '1';
    const consensus = loadConsensus({
      proofs: () => ({ miningProofs: [], computeProofs: [] })
    });
    const result = consensus.submitProposal(223, {
      proposer: 'someone', rewards: [{ miner: 'someone', amount: 10 }], total_work: 1, settled_jobs: 1, timestamp: 1
    });
    expect(result.accepted).toBe(true);
  });

  // ── Persistence / restart recovery ─────────────────────────────

  test('resolve persists the winner via the persist provider', () => {
    const persisted = [];
    const consensus = loadConsensus({ persist: (epoch, winner) => persisted.push({ epoch, winner }) });
    const rewards = [{ miner: 'x', amount: 5 }];

    consensus.submitProposal(230, { proposer: 'a', rewards, total_work: 1, settled_jobs: 1, timestamp: 1 });
    consensus.submitProposal(230, { proposer: 'b', rewards, total_work: 1, settled_jobs: 1, timestamp: 2 });

    expect(persisted).toHaveLength(1);
    expect(persisted[0].epoch).toBe(230);
    expect(persisted[0].winner.proposer).toBe('a');
  });

  test('FINALIZATION_CONSENSUS ledger entry restores resolved state after restart', () => {
    // Fresh module simulates a restarted process with empty in-memory state.
    // Load it first: jest.resetModules gives it its own stateStore instance,
    // so the entry must be applied to THAT instance.
    const consensus = loadConsensus();
    const stateStore = require('../src/chain/stateStore');
    stateStore.applyEntry({
      type: 'FINALIZATION_CONSENSUS',
      epoch: 231,
      timestamp: 1234,
      consensus_data: {
        epoch_number: 231,
        proposer: 'survivor',
        consensus_hash: 'abc123',
        total_work: 42,
        consensus_nodes: 2,
        consensus_proposals: 3,
        rewards: [{ miner: 'survivor', amount: 243 }]
      }
    });

    expect(consensus.isResolved(231)).toBe(true);
    const winner = consensus.getWinner(231);
    expect(winner.proposer).toBe('survivor');
    expect(winner.consensus_hash).toBe('abc123');
    expect(winner.restored_from_chain).toBe(true);

    // Late proposals for a chain-resolved epoch are rejected
    const late = consensus.submitProposal(231, {
      proposer: 'latecomer', rewards: [{ miner: 'latecomer', amount: 1 }], total_work: 1, settled_jobs: 1, timestamp: 9
    });
    expect(late.accepted).toBe(false);
    expect(late.consensus).toBe(true);
    expect(late.winner.proposer).toBe('survivor');
  });

  // ── Gap-fill: replay, empty epochs, hash mismatch, cleanup, min sources ──

  test('replay across epochs: same proposals resolve independently per epoch', () => {
    const consensus = loadConsensus();
    const rewards = [{ miner: 'x', amount: 5 }];

    for (const epoch of [240, 241, 242]) {
      consensus.submitProposal(epoch, { proposer: 'a', rewards, total_work: 1, settled_jobs: 1, timestamp: 1 });
      consensus.submitProposal(epoch, { proposer: 'b', rewards, total_work: 1, settled_jobs: 1, timestamp: 2 });
      expect(consensus.isResolved(epoch)).toBe(true);
    }
    expect(consensus.getWinner(240).proposer).toBe('a');
    expect(consensus.getWinner(242).proposer).toBe('a');
  });

  test('empty epoch (no rewards) still resolves', () => {
    const consensus = loadConsensus();
    const result1 = consensus.submitProposal(243, { proposer: 'a', rewards: [], total_work: 0, settled_jobs: 0, timestamp: 1 });
    expect(result1.accepted).toBe(true);
    const result2 = consensus.submitProposal(243, { proposer: 'b', rewards: [], total_work: 0, settled_jobs: 0, timestamp: 2 });
    expect(result2.consensus).toBe(true);
    expect(consensus.getWinner(243).rewards).toEqual([]);
  });

  test('blockProposal-style hash matches hashRewards output', () => {
    const consensus = loadConsensus();
    const rewards = [{ miner: 'm1', amount: 100.5 }, { miner: 'm2', amount: 42 }];
    const hash = consensus.hashRewards(rewards, 142.5, 2, 244);

    consensus.submitProposal(244, { proposer: 'm1', rewards, total_work: 142.5, settled_jobs: 2, timestamp: 1 });
    consensus.submitProposal(244, { proposer: 'm2', rewards, total_work: 142.5, settled_jobs: 2, timestamp: 2 });

    expect(consensus.getWinner(244).consensus_hash).toBe(hash);
    // A proposal with tampered total_work produces a different hash — no false grouping
    expect(consensus.hashRewards(rewards, 9999, 2, 244)).not.toBe(hash);
  });

  test('memory cleanup keeps only the last 10 epochs', () => {
    const consensus = loadConsensus();
    const rewards = [{ miner: 'x', amount: 1 }];
    for (let epoch = 300; epoch < 315; epoch++) {
      consensus.submitProposal(epoch, { proposer: 'a', rewards, total_work: 1, settled_jobs: 1, timestamp: 1 });
      consensus.submitProposal(epoch, { proposer: 'b', rewards, total_work: 1, settled_jobs: 1, timestamp: 2 });
    }
    // Oldest epochs evicted from memory (and nothing persisted in this test)
    expect(consensus.getProposals(300)).toHaveLength(0);
    expect(consensus.getProposals(314)).toHaveLength(2);
  });

  test('minimum distinct source count blocks resolution until met', () => {
    jest.useFakeTimers();
    process.env.HONE_PROPOSAL_WINDOW_MS = '100';
    process.env.HONE_MIN_CONSENSUS_PEERS = '2';
    const consensus = loadConsensus();
    const rewards = [{ miner: 'x', amount: 1 }];

    consensus.submitProposal(250, { proposer: 'a', rewards, total_work: 1, settled_jobs: 1, timestamp: 1 }, 'self');
    jest.advanceTimersByTime(100);
    expect(consensus.isResolved(250)).toBe(false);

    consensus.submitProposal(250, { proposer: 'b', rewards, total_work: 1, settled_jobs: 1, timestamp: 2 }, '203.0.113.7');
    expect(consensus.isResolved(250)).toBe(true);
  });
});
