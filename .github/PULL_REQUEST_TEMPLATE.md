## Description
<!-- Clear summary of changes. Link related issues. -->

## Change Type
- [ ] Bug fix
- [ ] New feature
- [ ] Consensus/Epoch logic
- [ ] Cryptography/Proof verification
- [ ] P2P/Networking
- [ ] Emergency/Governance pipeline
- [ ] Documentation/Compliance
- [ ] DevOps/CI/CD

## Security & Compliance Checklist
- [ ] `npm test` passes locally on modified modules
- [ ] No hardcoded secrets, keys, or credentials in diff
- [ ] New endpoints documented in `docs/api/`
- [ ] Consensus changes include unit + integration tests
- [ ] P2P messages use Ed25519 signature + nonce + timestamp
- [ ] Guardian multisig/timelock bypass not introduced

## Testing Evidence
```bash
# Paste npm test output or CI run link
```

## Emergency/Hotfix Path
- [ ] This PR is a security/emergency hotfix
- [ ] Bypasses normal review? If yes: justification + 2/5 guardian approval attached

---
*PRs touching consensus, cryptography, or emergency logic require explicit security sign-off before merge.*
