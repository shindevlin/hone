# Ferryman

This project uses Ferryman (https://github.com/estejosh/ferryman),
licensed under the Ferryman Source-Available License.

## Using the channel

Project id: `hone`. Channel folder: `X:\ferry\comms\hone-ferryman` (identity in `.ferryman/bridge.toml`; `.ferryman/` is never committed).

- `ferry channel status` shows the channel for this directory and its state.
- `ferry channel inbox` reads messages addressed to this agent; `ferry channel send` writes one.
- `ferry channel order` issues work; `ferry channel agents` lists who is taking part.
- Workers do not push. Finished commits go in with `ferry work hand-in`; the head accepts and pushes.
- This repo's remote is Shin's (shindevlin/hone): never push from this machine.
- Paths: see "Where Hone lives" in CLAUDE.md.