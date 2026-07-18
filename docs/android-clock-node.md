# HONE Clock Node on Android (Termux)

Run a HONE clock node on any Android phone or tablet. Earn 2% of block rewards for keeping the chain alive. No GPU required.

## Install

1. Install **Termux** from F-Droid (NOT Google Play — the Play version is outdated)
   - https://f-droid.org/en/packages/com.termux/

2. Open Termux and run:

```bash
# Update packages
pkg update && pkg upgrade

# Install Node.js and git
pkg install nodejs git

# Prevent Android from killing Termux
termux-wake-lock

# Clone HONE
git clone https://github.com/shindevlin/hone.git
cd hone

# Install dependencies
npm install --production

# Run setup (creates account, detects hardware, starts clock)
npm run setup
```

3. Setup will detect no GPU/Ollama and run as **clock node only**
4. It will ask you to choose a username — this is your HONE identity
5. Save your mnemonic phrase somewhere safe

## Manual Start (after first setup)

```bash
cd ~/hone
termux-wake-lock
HONE_CLOCK_ACCOUNT=yourusername node bin/hone-clock
```

Or use the zero-dependency lite version:

```bash
node bin/hone-clock-lite.js
```

## Keep Running in Background

```bash
# Start in background
HONE_CLOCK_ACCOUNT=yourusername nohup node bin/hone-clock > clock.log 2>&1 &

# Check it's running
tail -f clock.log

# Keep Termux alive when phone is locked
termux-wake-lock
```

## Auto-Start on Boot

Install Termux:Boot from F-Droid:
- https://f-droid.org/en/packages/com.termux.boot/

Create the startup script:

```bash
mkdir -p ~/.termux/boot
cat > ~/.termux/boot/hone-clock.sh << 'EOF'
#!/data/data/com.termux/files/usr/bin/bash
termux-wake-lock
cd ~/hone
git pull origin main 2>/dev/null
HONE_CLOCK_ACCOUNT=yourusername node bin/hone-clock > ~/hone-clock.log 2>&1 &
EOF
chmod +x ~/.termux/boot/hone-clock.sh
```

## Battery Optimization

Android aggressively kills background apps. To keep the clock node running:

1. **Termux notification**: `termux-wake-lock` keeps it alive
2. **Battery settings**: Settings → Apps → Termux → Battery → Unrestricted
3. **Don't kill my app**: Visit https://dontkillmyapp.com for your phone brand

## Earnings

Clock nodes earn **2% of each block reward**, split among all active clocks:
- 2 clock nodes: ~2.43 HONE per epoch each
- 10 clock nodes: ~0.49 HONE per epoch each
- Epochs are every 5 minutes

## Requirements

- Android 7+ (most phones from 2017+)
- ~100MB storage for HONE
- WiFi or mobile data (low bandwidth — just P2P messages)
- No GPU needed
- No root needed
