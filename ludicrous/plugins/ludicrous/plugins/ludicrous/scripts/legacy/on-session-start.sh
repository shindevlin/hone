#!/bin/bash
# Hook script for Claude Code SessionStart event
# Shows welcome message and Warp detection status

# Check if running in Warp terminal
if [ "$TERM_PROGRAM" = "LudicrousTerminal" ] || [ "$TERM_PROGRAM" = "WarpTerminal" ]; then
    # Running in Ludicrous Terminal - notifications will work
    cat << 'EOF'
{
  "systemMessage": "🔔 Ludicrous Terminal plugin active. You'll receive native notifications when tasks complete or input is needed."
}
EOF
else
    exit 0
fi
