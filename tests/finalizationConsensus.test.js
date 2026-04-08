describe('finalizationConsensus', () => {
  function loadConsensus() {
    jest.resetModules();
    return require('../src/chain/finalizationConsensus');
  }

  beforeEach(() => {
    jest.useRealTimers();
    delete process.env.BTCPC_PROPOSAL_WINDOW_MS;
    delete process.env.BTCPC_PROPOSAL_TIMEOUT_MS;
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
    process.env.BTCPC_PROPOSAL_WINDOW_MS = '100';
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
});
