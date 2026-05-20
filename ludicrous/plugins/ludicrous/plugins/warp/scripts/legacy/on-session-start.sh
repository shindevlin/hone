#!/bin/bash
# Hook script for Claude Code SessionStart event
# Shows welcome message and Ludicrous Terminal detection status

# Check if running in Ludicrous Terminal
if [ "$TERM_PROGRAM" = "WarpTerminal" ]; then
    # Running in Ludicrous Terminal - notifications will work
    cat << 'EOF'
{
  "systemMessage": "🔔 Ludicrous Terminal plugin active. You'll receive native notifications when tasks complete or input is needed."
}
EOF
else
    exit 0
fi
