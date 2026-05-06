#!/bin/bash
# One-time setup: open Firefox as each persona and log into social platforms.
# Sessions are saved to the persona's Firefox profile and reused by the poster.
#
# Run this ONCE per persona. After completing, the poster will be logged in
# and can post automatically without intervention.

set -e

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PROFILES_DIR="$REPO/firefox-profiles"

PERSONA="${1:-shindevlin}"
PROFILE_DIR="$PROFILES_DIR/$PERSONA"

declare -A PLATFORMS_SHIN=(
  ["twitter"]="https://x.com/login"
  ["reddit"]="https://www.reddit.com/login"
  ["hn"]="https://news.ycombinator.com/login"
  ["substack"]="https://substack.com/sign-in"
)
declare -A PLATFORMS_NATOSHI=(
  ["twitter"]="https://x.com/login"
  ["reddit"]="https://www.reddit.com/login"
)

mkdir -p "$PROFILE_DIR"

echo "============================================================"
echo "  Account setup for persona: $PERSONA"
echo "  Firefox profile: $PROFILE_DIR"
echo "============================================================"
echo ""
echo "Firefox will open. Log into each platform as $PERSONA."
echo "Sessions are saved automatically to the profile."
echo ""

if [ "$PERSONA" = "shindevlin" ]; then
  for PLATFORM in "${!PLATFORMS_SHIN[@]}"; do
    URL="${PLATFORMS_SHIN[$PLATFORM]}"
    echo "Opening $PLATFORM → $URL"
    echo "  Log in, then come back here and press Enter to continue."
    firefox --profile "$PROFILE_DIR" --new-tab "$URL" &
    read -r -p "  Press Enter when done with $PLATFORM... "
    echo ""
  done
elif [ "$PERSONA" = "natoshisakamoto" ]; then
  for PLATFORM in "${!PLATFORMS_NATOSHI[@]}"; do
    URL="${PLATFORMS_NATOSHI[$PLATFORM]}"
    echo "Opening $PLATFORM → $URL"
    echo "  Log in, then come back here and press Enter to continue."
    firefox --profile "$PROFILE_DIR" --new-tab "$URL" &
    read -r -p "  Press Enter when done with $PLATFORM... "
    echo ""
  done
else
  echo "Unknown persona: $PERSONA. Use: shindevlin | natoshisakamoto"
  exit 1
fi

echo "============================================================"
echo "  Setup complete for $PERSONA."
echo "  Profile saved at: $PROFILE_DIR"
echo "  The poster will reuse these sessions automatically."
echo "============================================================"
