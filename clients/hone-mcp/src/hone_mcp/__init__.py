"""hone-mcp — Model Context Protocol server for the HONE node's local HTTP API.

This package turns a running hone-node's local HTTP API into MCP tools, so
an agentic client (Claude Desktop, etc.) can read chain/account/sensor/clock
state and drive the zero-key-custody onboarding flows (account creation,
clock self-registration, testnet opt-in) conversationally.

See README.md for scope, security model, and what is deliberately NOT
implemented (chain/protocol-level signing for pre-existing keyed accounts).
"""

__version__ = "0.1.0"
