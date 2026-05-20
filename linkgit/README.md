# linkgit

Ed25519 identity layer for BTCPC — links a Git commit signature (or SSH key)
to a chain posting key. Enables developer-native auth without passwords or JWTs.

A signed git commit proves key ownership; the node verifies the signature and
grants the corresponding chain account read/write access.

## Status

Core auth flow implemented in `rust/btcpc-node` (LinkGit entry type).
This directory will hold the standalone CLI and library.
