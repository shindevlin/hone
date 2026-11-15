# License acceptance gate

HONE is licensed under the Usufruct License (UFL) 3.8, Operational Scope:
Decentralized (`LicenseRef-UFL-3.8-D`). The full text is in [`LICENSE`](../LICENSE);
third-party code that keeps its own license is listed in [`NOTICE`](../NOTICE).

Under the Decentralized scope, use is free and nothing is paid. The scope's
dispute process (exclusive online ICC arbitration seated in California), its
mutual liability limit (US$1 in value, payable in money or HONE), and its
own-risk terms for forks and bypassers take effect only through an affirmative
acceptance step that the software presents before first use (UFL Section 9).
The software must not run until that step is done. Software that ships without
the step is licensed under the Unconditional scope instead.

The step is the `hone-license` crate (`rust/hone-license`). Every HONE Rust
binary calls it before doing any work.

## What happens at startup

1. **Record.** If `{data_dir}/license-acceptance.json` names UFL 3.8,
   Decentralized, the binary starts.
2. **Explicit acceptance.** Otherwise, if `--accept-ufl 3.8` is on the command
   line or `HONE_ACCEPT_LICENSE=UFL-3.8-D` is set, that is acceptance: the
   record is written and the binary starts. Any other value of either is
   refused and the binary exits; a typo never falls through to "run anyway".
   An empty variable counts as unset.
3. **Prompt.** Otherwise, if stdin and stderr are a terminal, the binary
   shows the version and scope, the arbitration terms, the US$1 limit, and the
   own-risk terms, and asks you to type `I accept`. `show` prints the full
   license first. Anything else exits without starting.
4. **Refuse.** Otherwise (no terminal: a service, a container, a pipe) the
   binary exits with status **3**, printing the terms and the variable and
   flag that accept them.

The gate never accepts on your behalf. Nothing in the repository sets
`HONE_ACCEPT_LICENSE`.

If the record names an older version or another scope (for example after HONE
moves to a later UFL release), the binary asks again (or exits 3 when headless).

## Accepting without a terminal

Setting the variable or passing the flag **is the act of accepting the
license**. Read [`LICENSE`](../LICENSE) first.

| Where | How |
|---|---|
| Shell | `HONE_ACCEPT_LICENSE=UFL-3.8-D hone-node` or `hone-node --accept-ufl 3.8` |
| systemd | Run `hone-license accept` once as the service user with the same `HONE_DATA_DIR`, or uncomment `Environment="HONE_ACCEPT_LICENSE=UFL-3.8-D"` in the unit. Units carry `RestartPreventExitStatus=3` so an unaccepted service does not restart-loop. |
| Docker Compose | Put `HONE_ACCEPT_LICENSE=UFL-3.8-D` in your shell or `.env`. Compose passes it through; no value is set in the compose files. |
| Docker | `docker run -e HONE_ACCEPT_LICENSE=UFL-3.8-D ...` The image does not bake acceptance in. |

After one successful acceptance the record lets later runs start without the
variable, as long as they use the same data directory.

## The `hone-license` tool

```
hone-license            # same as accept
hone-license accept     # run the acceptance step now (prompt, flag, or variable)
hone-license status     # exit 0 if this data dir has accepted, 3 if not
hone-license show       # print the full license text
hone-license statement  # print the Ruling License line
```

## Record file

`{data_dir}/license-acceptance.json`, where `data_dir` is `HONE_DATA_DIR` or
`~/.hone` (hone-node uses its own configured data directory, which follows the
same rule):

```json
{
  "license": "UFL",
  "version": "3.8",
  "scope": "Decentralized",
  "accepted_at": "2026-10-05T23:59:59Z",
  "binary": "hone-node"
}
```

That is all it holds: no user name, machine id, IP, wallet, or account. It is
written only to the local data directory, as UFL Section 9 requires. Nothing is
sent over the network or written on chain.

## Binaries covered

The gate runs first in `main` (hone-node: right after `Config::from_env()`,
before the port probe, the instance lock, and `wallet::init`):

| Crate | Binary |
|---|---|
| hone-node | `hone-node` |
| hone-p2p | `hone-p2p` |
| hone-market | `hone-market` |
| hone-contract-runtime | `hone-contract-runtime` |
| linkgit | `git-remote-linkgit` |
| hone-cli | `hone` |
| hone-sdk | `hone`, `hone-keycheck` |
| hone-tui | `hone-tui` |
| hone-gui | `hone-gui` |
| hone-wallet-create | `hone-wallet-create` |
| hone-orchestrator | `hone-orchestrator` |
| hone-gnss-capture | `hone-gnss-capture` |
| hone-ble-tracker | `hone-ble-tracker` |
| hone-hivefs-adapter | `hone-hivefs-adapter` |
| hone-api-catalog | `hone-api-catalog` |
| wiiv | `wiiv` |
| hone-bot | `hone-bot` |
| betchu-bot | `betchu-bot` |

Binaries with their own argument parser read arguments through
`hone_license::args()` / `args_os()`, which remove `--accept-ufl <v>` so the
flag never reaches the parser.

Not gated (no standalone HONE runtime): libraries (`hone-types`,
`chain-core`, `hone-provision`, `hone-contract-sdk` and its example
contracts), the `uniffi-bindgen` build tool in `hone-android`, and the Android
`cdylib` miner libraries, which the Android app gates. The deprecated Node.js
prototype (`src/`, `bin/`) is not gated.

GUI binaries (`hone-gui`, `hone-wallet-create`) show a first-run license window
instead of the terminal prompt (`rust/hone-license-egui`); the real app is
built only after "I accept", and "Decline and quit" exits 3.

The desktop app, the Android app, the Flipper apps, and the installers have
their own first-run step with the same terms and record. See
[LICENSE_GATE_CLIENTS.md](LICENSE_GATE_CLIENTS.md).

## Contributor acknowledgement (CLA)

Contributions are governed by UFL Section 4. Pull requests are checked by
`.github/workflows/cla.yml`: the author posts the sign comment from
[`CLA.md`](../CLA.md) once, and the record goes to `signatures/cla.json` on the
`cla-signatures` branch. See [CONTRIBUTING.md](../CONTRIBUTING.md).

## Using the gate in a new binary

```toml
[dependencies]
hone-license = { path = "../hone-license" }
```

```rust
fn main() {
    hone_license::require_acceptance_or_exit(&hone_license::default_data_dir());
    // If you parse arguments, use hone_license::args() / args_os().
    ...
}
```

`require_acceptance(data_dir, Mode::Auto | Mode::Interactive | Mode::Headless)`
returns the result instead of exiting. Use `Mode::Headless` for a process whose
stdin is a protocol pipe.

## Release statement (UFL Section 1C)

Each HONE Release states its Ruling License where users get it:

```
UFL 3.8, Operational Scope: Decentralized
```

`.github/workflows/release-node.yml` writes this line (with
`LicenseRef-UFL-3.8-D`) into every release body and attaches `LICENSE` and
`NOTICE`. The UFL repository's generator prints the line:
`sh generate.sh -r -s decentralized -k HONE`. When HONE moves to a later UFL
version or scope, update `LICENSE`, the constants in `rust/hone-license`, and
the workflow line in the same commit as the tag. Releases already published
keep the Ruling License they stated.
