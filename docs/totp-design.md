# Per-User TOTP 2FA Design

## Current State
Global `TWOFA_TOKEN` env var — security theater. One shared secret for all users.

## Target State
Each user has their own TOTP secret (Google Authenticator / Authy compatible).

## Implementation

### User Model Changes
```javascript
{
  totpSecret: String,      // encrypted, base32-encoded TOTP secret
  totpEnabled: Boolean,    // false by default
  totpBackupCodes: [String] // 8 one-time backup codes
}
```

### Setup Flow
1. User calls `/enable-2fa` in bot or webapp
2. Server generates random TOTP secret
3. Returns QR code URL (otpauth://totp/HONE:username?secret=XXX&issuer=HONE)
4. User scans with Google Authenticator / Authy
5. User submits the current 6-digit code to verify
6. Server stores encrypted secret + generates 8 backup codes
7. 2FA is active — required for: transfers, staking, key changes

### Verification
- Every sensitive action requires `totp_code` parameter
- Server verifies using `speakeasy` or built-in crypto HMAC-SHA1
- 30-second window with +-1 step tolerance
- Backup codes: single-use, each consumed on verification

### What Requires 2FA (when enabled)
- Token transfers
- Staking / unstaking
- Delegation changes
- Key rotation
- 2FA disable (requires owner key + TOTP)

### What Does NOT Require 2FA
- Balance checks
- Transaction history
- Inference requests (project API key is auth)
- Heartbeat
- Posting key operations

### Dependencies
- `speakeasy` npm package (TOTP generation/verification)
- `qrcode` npm package (QR code for setup)
- Or pure crypto implementation with Node.js built-in HMAC

### Timeline
Post-launch. The key hierarchy (owner/active/posting/memo) provides strong security without TOTP. TOTP adds defense-in-depth for users who want it.
