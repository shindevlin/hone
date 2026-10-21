<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph

**IMPORTANT: This project has a knowledge graph. ALWAYS use the
code-review-graph MCP tools BEFORE using Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives
you structural context (callers, dependents, test coverage) that file
scanning cannot.

### When to use graph tools FIRST

- **Exploring code**: `semantic_search_nodes` or `query_graph` instead of Grep
- **Understanding impact**: `get_impact_radius` instead of manually tracing imports
- **Code review**: `detect_changes` + `get_review_context` instead of reading entire files
- **Finding relationships**: `query_graph` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview` + `list_communities`

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
|------|----------|
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context` | Need source snippets for review — token-efficient |
| `get_impact_radius` | Understanding blast radius of a change |
| `get_affected_flows` | Finding which execution paths are impacted |
| `query_graph` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes` | Finding functions/classes by name or keyword |
| `get_architecture_overview` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes` for code review.
3. Use `get_affected_flows` to understand impact.
4. Use `query_graph` pattern="tests_for" to check coverage.

## Where Hone lives

- **Code:** this repo, `X:\ferry\repos\hone` (WSL `/mnt/x/ferry/repos/hone`). Set `HONE_HOME` to it; scripts fall back to `git rev-parse --show-toplevel`. Never hardcode a drive path.
- **Data:** `HONE_DATA_ROOT` = `X:\ferry\hone-data` (WSL `/mnt/x/ferry/hone-data`): hone-models, hone-inference-data (sensitive), hone-control, hone-tools, hone-export-dist, hone-gui-dist, hone-seal-dist, hone-worker-adapter, HONE_SECURITY_BACKUPS_LOCAL_ONLY (sensitive), OpenClaw HONE. Config lives in `X:\ferry\hone-data\hone-home.env`.
- **Channel:** `X:\ferry\comms\hone-ferryman` via `.ferryman/bridge.toml` (project `hone`). Read `FERRYMAN.md` for how to use it. This is not hone-bridge (a separate project).
- **Old checkout:** `X:\hone` is retired to `X:\ferry\_retired\hone`. Its 51 branches are local branches here and `legacy/*` remote refs. Local main follows origin/main; the old btcpc lineage shares no history with it.
- Ask `ferry where hone` once available.
- Remotes: push access to shindevlin/hone is Shin's; do not push from this machine.