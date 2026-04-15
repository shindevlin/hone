# BTCPC External Auditor RFP: SOC2 Type II & ISO 27001 Readiness

## 1. Executive Summary
BTCPC is a decentralized blockchain protocol enabling Proof-of-Compute consensus via AI inference. We seek a qualified third-party auditor to assess our security controls, operational resilience, and compliance posture against SOC2 Type II (Security, Availability) and ISO/IEC 27001:2022 standards.

**Compliance Targets:**
- SOC2 Type II (6-month observation window)
- ISO/IEC 27001:2022 Certification (pre-assessment + Stage 1)

## 2. System Architecture & Boundaries
| Component | Description | In Scope? |
|-----------|-------------|-----------|
| P2P Consensus Layer | Ed25519-signed epoch proposals, VRF proposer rotation, work verification | ✅ |
| Guardian Governance | 3/5 multisig parameter upgrades, timelock executor, emergency pause/resume | ✅ |
| Node Infrastructure | Node.js runtime, stateStore (in-memory + disk snapshots), Prometheus exporters | ✅ |
| External Dependencies | Ollama inference runtime, Docker/Podman deployment, GitHub CI/CD | ✅ |
| Cross-Chain Bridges | Polygon/BSC Solidity contracts (pre-audit only) | 🟡 (Limited scope) |
| User Wallets/Keys | End-user AI prompt submission, local mining nodes | ❌ (Out of scope) |

## 3. Required Control Evidence
- `docs/compliance/SOC2_ISO27001_MAPPING.md` (pre-mapped controls)
- Emergency drill logs: `docs/operations/reports/<YEAR>/`
- Guardian proposal history: `/consensus/params/history` endpoint
- CI/CD security gates: `.github/workflows/`

## 4. Audit Methodology Expectations
| Phase | Activities | Timeline |
|-------|-----------|----------|
| Scoping | Control validation, evidence collection planning | Week 1-2 |
| Fieldwork | Testnet observation, CI review, emergency drill validation | Week 3-8 |
| Reporting | Draft findings, management response, final SOC2 report | Week 9-12 |
| ISO 27001 Pre-Assessment | Gap analysis, statement of applicability review | Week 13 |

**Required Access:** Read-only GitHub repo access, testnet node endpoint (`GET /health`, `/consensus/epoch/current`), Prometheus/Grafana read-only credentials, emergency drill logs.

## 5. Deliverables
1. SOC2 Type II Report (Security + Availability)
2. ISO/IEC 27001 Statement of Applicability (SoA) Gap Assessment
3. Management Letter with remediation roadmap
4. Raw evidence index mapped to BTCPC control IDs
5. Executive summary suitable for board/investor distribution

## 6. Vendor Requirements
- Active SOC2/ISO 27001 accreditation (AICPA, UKAS, or equivalent)
- Prior experience auditing decentralized/blockchain protocols
- Demonstrated capability in automated evidence validation (CI/CD, Prometheus, GitOps)
- Dedicated audit team ≤3 FTE, with blockchain security specialist

## 7. Submission & Evaluation
**Proposal Deadline:** `<DATE>`
**Submission Format:** PDF + pricing breakdown + case studies (2 max)
**Evaluation Criteria:**
- Blockchain/consensus protocol experience (40%)
- Automated evidence methodology (25%)
- Timeline & resource allocation (20%)
- Cost & reporting flexibility (15%)

**Contact:** `compliance@btcpc.net` | `security@btcpc.net`
**Repo:** `https://github.com/shindevlin/btcpc`

---
*RFP Version: 1.0 — Issued By: BTCPC Security & Compliance Working Group*
