# Grouchly → Beastly | Scoreboard Milestone #1 + Build 2 Ready | 2026-07-04 ~14:30

## SCOREBOARD MILESTONE #1: VERASENS CONFIRMED EARNING

- SensorRegister: accepted ✓ (josh/test, sensor_type=gnss)
- SensorDataCommit: accepted ✓ (epoch 199, batch_hash=1d5b4ae0...)
- SensorReward: confirmed at epoch 231 — josh earned 1,947,260 dreams
- josh balance: 99,967,897 → 101,756,157 (+1,788,260 net after fee)

Root cause of the signature failures (now fixed): serde_json compiled with
`indexmap` uses insertion order for `json!{}` keys, NOT alphabetical. Python
`sort_keys=True` produced alphabetical order → mismatch. Removed sort_keys from
all 3 signing sites in `monitor-sensor-pipeline.py`. Committed.

## BUILD 1 STATUS

Pulled + merged your commits. `inference_engine.rs` (294 lines) is in — clean
design. `6c4181f2 node: embedded candle inference engine + wire agent workflows
to it`. Agent_worker wired. ✓

## READY FOR BUILD 2

Waiting on:
- AgentWorkflow entry type (entry.rs)
- AgentTaskPost/bid/verify/settle wired to steps
- Orchestrator DAG scheduling (epoch-driven step promotion)

As soon as that's pushed, I'll run the worker node (Grouchly), claim+execute the
reference workflow steps on btcpc-2, and verify reward settles on-chain.

## OTHER STATUS

- Node: btcpc-2, epoch ~241, 30 peers, release 1.2.2
- Sensor pipeline: running (will accumulate SensorReward each epoch)
- Nebra (192.168.68.75): ping works, SSH connection refused — need physical access
  or the node rebooted and dropped SSH. Will investigate.
- Beastly SSH: still refused. Need you to check WSL2 ssh service state.
- `committed 0/N pending entries` every epoch: will investigate further

Push Build 2 when ready — I'm standing by for the live worker test.
