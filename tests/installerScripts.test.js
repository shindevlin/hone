"use strict";

/**
 * installerScripts.test.js
 *
 * Verifies that the three HONE installer scripts follow the self-heal
 * rule: auto-recover on every failure, never ask the user to do something.
 *
 * Checks:
 *   - .bat and .ps1 use CRLF line endings
 *   - .bat is ASCII-only (no bytes > 127)
 *   - install.sh uses LF line endings
 *   - .bat does NOT contain a bare `pause >nul` at end-of-file (watchdog loop instead)
 *   - .bat does NOT contain unguarded `exit /b 1`
 *   - .ps1 does NOT contain `Read-Host "Press Enter to exit"`
 *   - .ps1 does NOT contain unguarded `exit 1`
 *   - install.sh does NOT contain bare `exit 1` (should be in retry loops)
 *   - Each file contains a retry/loop keyword
 */

const fs = require('fs');
const path = require('path');

const WEBSITE = path.resolve(__dirname, '../website');
const BAT  = path.join(WEBSITE, 'hone-start.bat');
const PS1  = path.join(WEBSITE, 'hone-start.ps1');
const SH   = path.join(WEBSITE, 'install.sh');

describe('installer scripts — encoding', () => {
  test('.bat file exists', () => {
    expect(fs.existsSync(BAT)).toBe(true);
  });

  test('.ps1 file exists', () => {
    expect(fs.existsSync(PS1)).toBe(true);
  });

  test('install.sh exists', () => {
    expect(fs.existsSync(SH)).toBe(true);
  });

  test('.bat has CRLF line endings', () => {
    const buf = fs.readFileSync(BAT);
    // Must contain at least one \r\n
    let hasCRLF = false;
    for (let i = 0; i < buf.length - 1; i++) {
      if (buf[i] === 0x0d && buf[i + 1] === 0x0a) { hasCRLF = true; break; }
    }
    expect(hasCRLF).toBe(true);
    // Must NOT contain bare LF (i.e., LF not preceded by CR)
    let hasBareLF = false;
    for (let i = 0; i < buf.length; i++) {
      if (buf[i] === 0x0a && (i === 0 || buf[i - 1] !== 0x0d)) {
        hasBareLF = true;
        break;
      }
    }
    expect(hasBareLF).toBe(false);
  });

  test('.ps1 has CRLF line endings', () => {
    const buf = fs.readFileSync(PS1);
    let hasCRLF = false;
    for (let i = 0; i < buf.length - 1; i++) {
      if (buf[i] === 0x0d && buf[i + 1] === 0x0a) { hasCRLF = true; break; }
    }
    expect(hasCRLF).toBe(true);
    let hasBareLF = false;
    for (let i = 0; i < buf.length; i++) {
      if (buf[i] === 0x0a && (i === 0 || buf[i - 1] !== 0x0d)) {
        hasBareLF = true;
        break;
      }
    }
    expect(hasBareLF).toBe(false);
  });

  test('.bat is ASCII only (no bytes > 127)', () => {
    const buf = fs.readFileSync(BAT);
    const nonAscii = [];
    for (let i = 0; i < buf.length; i++) {
      if (buf[i] > 127) nonAscii.push({ offset: i, byte: buf[i] });
    }
    expect(nonAscii).toHaveLength(0);
  });

  test('install.sh has LF line endings (no CR bytes)', () => {
    const buf = fs.readFileSync(SH);
    let hasCR = false;
    for (let i = 0; i < buf.length; i++) {
      if (buf[i] === 0x0d) { hasCR = true; break; }
    }
    expect(hasCR).toBe(false);
  });
});

describe('installer scripts — no dead-end exits', () => {
  let batText, ps1Text, shText;

  beforeAll(() => {
    batText = fs.readFileSync(BAT, 'utf8');
    ps1Text = fs.readFileSync(PS1, 'utf8');
    shText  = fs.readFileSync(SH,  'utf8');
  });

  test('.bat does NOT end with bare "pause >nul" as the last user-visible line', () => {
    // The old pattern was: error handler -> echo [ERROR] -> goto END -> END: -> pause >nul
    // New pattern uses :MAIN_LOOP. The file should not have pause >nul near end of file
    // (last 500 chars is a reasonable proxy for "at end-of-file")
    const tail = batText.slice(-500).toLowerCase();
    expect(tail).not.toMatch(/pause\s*>nul/);
  });

  test('.bat does NOT contain unguarded "exit /b 1"', () => {
    // exit /b 1 should not appear in the bat at all — failures loop back via MAIN_LOOP
    const lines = batText.split('\n');
    const exitLines = lines.filter(l => /exit\s*\/b\s*1/i.test(l) && !l.trim().startsWith('REM'));
    expect(exitLines).toHaveLength(0);
  });

  test('.ps1 does NOT contain "Read-Host \\"Press Enter to exit\\""', () => {
    expect(ps1Text).not.toMatch(/Read-Host\s+["']Press Enter to exit["']/i);
  });

  test('.ps1 does NOT contain unguarded "exit 1"', () => {
    // exit 1 should not appear outside of comments
    const lines = ps1Text.split('\n');
    const exitLines = lines.filter(l => /^\s*exit\s+1\b/.test(l) && !l.trim().startsWith('#'));
    expect(exitLines).toHaveLength(0);
  });

  test('install.sh does NOT contain bare "exit 1" outside retry loops', () => {
    // Bare exit 1 must not appear — use exit 0 or just let the retry handle it
    // We allow "exit 0" (graceful non-recoverable like "wrong OS") but not "exit 1"
    const lines = shText.split('\n');
    const exitLines = lines.filter(l => /^\s*exit\s+1\b/.test(l.replace(/#.*$/, '')));
    expect(exitLines).toHaveLength(0);
  });
});

describe('installer scripts — retry/loop keywords present', () => {
  let batText, ps1Text, shText;

  beforeAll(() => {
    batText = fs.readFileSync(BAT, 'utf8');
    ps1Text = fs.readFileSync(PS1, 'utf8');
    shText  = fs.readFileSync(SH,  'utf8');
  });

  test('.bat contains :MAIN_LOOP', () => {
    expect(batText).toMatch(/:MAIN_LOOP/);
  });

  test('.ps1 contains "do {" retry loop', () => {
    expect(ps1Text).toMatch(/do\s*\{/);
  });

  test('.ps1 contains "while ($keepRetrying)"', () => {
    expect(ps1Text).toMatch(/while\s*\(\$keepRetrying\)/);
  });

  test('install.sh contains "while true" retry loop', () => {
    expect(shText).toMatch(/while\s+true/);
  });

  test('install.sh contains exponential/sleep retry pattern', () => {
    // Should have sleep with a numeric arg for retries
    expect(shText).toMatch(/sleep\s+\d+/);
  });
});

describe('installer scripts — self-heal behaviors present', () => {
  let batText, ps1Text, shText;

  beforeAll(() => {
    batText = fs.readFileSync(BAT, 'utf8');
    ps1Text = fs.readFileSync(PS1, 'utf8');
    shText  = fs.readFileSync(SH,  'utf8');
  });

  test('.bat polls docker engine up to 10 minutes (120 * 5s iterations)', () => {
    expect(batText).toMatch(/1,1,120/);
  });

  test('.bat has guest username fallback via powershell guid', () => {
    expect(batText).toMatch(/guid.*NewGuid/i);
    expect(batText).toMatch(/guest-/);
  });

  test('.bat tries to download Docker Desktop installer', () => {
    expect(batText).toMatch(/Docker.*Desktop.*Installer/i);
  });

  test('.bat has exponential backoff delays (5, 15, 45, 120, 300)', () => {
    expect(batText).toMatch(/DL_WAIT=5/);
    expect(batText).toMatch(/DL_WAIT=15/);
    expect(batText).toMatch(/DL_WAIT=45/);
    expect(batText).toMatch(/DL_WAIT=120/);
    expect(batText).toMatch(/DL_WAIT=300/);
  });

  test('.ps1 has guest username fallback via NewGuid', () => {
    expect(ps1Text).toMatch(/guest-.*NewGuid/i);
  });

  test('.ps1 has exponential backoff delays array', () => {
    expect(ps1Text).toMatch(/5,\s*15,\s*45,\s*120,\s*300/);
  });

  test('.ps1 auto-launches Docker Desktop', () => {
    expect(ps1Text).toMatch(/Start-Process.*Docker/i);
  });

  test('.ps1 has $keepRetrying = $false to exit loop on success', () => {
    expect(ps1Text).toMatch(/\$keepRetrying\s*=\s*\$false/);
  });

  test('install.sh does NOT install MongoDB', () => {
    expect(shText).not.toMatch(/mongo/i);
    expect(shText).not.toMatch(/27017/);
  });

  test('install.sh installs Ollama silently via curl pipe sh', () => {
    expect(shText).toMatch(/ollama\.com\/install\.sh.*\|\s*sh/);
  });

  test('install.sh wraps hone-setup in a restart loop', () => {
    expect(shText).toMatch(/while\s+true[\s\S]*?hone-setup[\s\S]*?done/);
  });

  test('install.sh handles private repo gracefully (no exit 1)', () => {
    // Should print a message and exit 0 (not exit 1) when clone fails without token
    expect(shText).toMatch(/exit\s+0/);
  });

  test('install.sh uses GITHUB_TOKEN for private repo fallback', () => {
    expect(shText).toMatch(/GITHUB_TOKEN/);
  });
});
