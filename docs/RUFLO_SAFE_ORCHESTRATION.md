# Safe Ruflo Project Orchestration

## Decision
Ruflo is a development orchestration tool only. Do not install or initialize it inside the main BTCPC checkout, wallet directories, release directories, or any repo containing secrets.

The sandbox test showed Ruflo can create `.claude`, `.claude-flow`, `.mcp.json`, `AGENTS.md`, `.agents`, `.codex`, hooks, daemon config, MCP config, and permissive local profiles. That is too invasive for BTCPC production code.

## Safe Starting Workflow
Use one isolated sandbox per project.

```bash
export RUFLO_VERSION=3.7.0-alpha.20
export RUFLO_SANDBOX=/mnt/btcpc-storage/sandboxes/ruflo-projects/my-project

mkdir -p "$RUFLO_SANDBOX/home" \
         "$RUFLO_SANDBOX/npm-prefix" \
         "$RUFLO_SANDBOX/npm-cache" \
         "$RUFLO_SANDBOX/work"

HOME="$RUFLO_SANDBOX/home" \
npm_config_cache="$RUFLO_SANDBOX/npm-cache" \
npm install -g --prefix "$RUFLO_SANDBOX/npm-prefix" "ruflo@$RUFLO_VERSION"

cd "$RUFLO_SANDBOX/work"
git clone <project-url> project
cd project

HOME="$RUFLO_SANDBOX/home" \
PATH="$RUFLO_SANDBOX/npm-prefix/bin:$PATH" \
ruflo init --codex --minimal --no-global
```

## Operating Rules
- Use throwaway clones or disposable worktrees.
- Do not mount `~/.btcpc`, wallet files, signing keys, browser profiles, Proton/Camofox profiles, or release credentials.
- Do not use Ruflo auto-commit, auto-push, daemon, or full init modes until every generated hook/config is reviewed.
- Review diffs manually before copying patches back to the real repo.
- Prefer task prompts with tight file scopes.
- Treat generated `.agents/config.toml`, `.codex/config.toml`, `.mcp.json`, and hooks as untrusted until reviewed.

## Recommended First Use
Start with documentation or non-critical prototype work:

```bash
HOME="$RUFLO_SANDBOX/home" \
PATH="$RUFLO_SANDBOX/npm-prefix/bin:$PATH" \
ruflo status

HOME="$RUFLO_SANDBOX/home" \
PATH="$RUFLO_SANDBOX/npm-prefix/bin:$PATH" \
ruflo task --help
```

Then ask it to plan work in the sandbox clone. Do not let it operate on BTCPC mainline until its output has been reviewed and manually ported.

## BTCPC Boundary
Ruflo may help orchestrate coding projects. It must not become BTCPC's runtime, agent layer, wallet tooling, or production orchestrator.
