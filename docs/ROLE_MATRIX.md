# HONE Role Matrix

> HONE should run as one machine with many roles, not one process with many responsibilities.

## Core Rules

- One machine may run multiple HONE roles.
- One machine may run at most one instance of any given role.
- Each role should have its own process or service boundary.
- `hone-all` is a supervisor/launcher, not the runtime architecture.
- Mobile can keep the role boundaries logically separate in-app even when the OS constrains process layout.

## Role Matrix

Legend:

- `Yes` = good fit
- `Maybe` = feasible if the hardware and model size make sense
- `No` = not a sensible default

| Role | Phone | Raspberry Pi / Nebra | Laptop / Desktop | Server / Workstation | Flipper |
|------|-------|----------------------|-------------------|----------------------|---------|
| `hone-clock` | Yes | Yes | Yes | Yes | Maybe |
| `hone-mine` | Yes, small model | Maybe | Yes | Yes | No |
| `hone-storage` | No by default | Yes | Yes | Yes | No |
| `hone-verifier` | Maybe | Maybe | Yes | Yes | No |
| `hone-reviewer` | Maybe | Maybe | Yes | Yes | No |
| `hone-sensor` | Yes, with GPS and other phone sensors | Yes | Yes, if hardware exists | Yes, if hardware exists | Yes |
| `hone-gateway` | No | Maybe | Maybe | Maybe | No |

## Notes By Device

### Phone

- Good for wallet/UI, clock, and a smaller miner model.
- The miner should be logically separated from the rest of the app.
- Phone sensors are a real role: GPS, motion, location, and other on-device signals can be tied together as one sensor process.
- This is a mobile node, not a combined `hone-all` blob.

### Raspberry Pi / Nebra

- Best for lightweight edge roles.
- Can run whatever roles the device can physically support.
- Good candidates are clock, storage, verifier, reviewer, and sensor if attached hardware exists.
- Mining is possible only if the model and thermals make sense.

### Laptop / Desktop

- The most flexible default node.
- Can usually run clock, miner, storage, verifier, and reviewer as separate processes.
- Sensor and gateway roles are only relevant if the hardware is present.

### Server / Workstation

- Same role model as desktop, but usually with more headroom.
- Good default for multi-role deployments that need stronger uptime.

### Flipper

- Likely sensor first.
- Maybe clock if the deployment wants it.
- Not a default miner, storage, verifier, or reviewer device.

## Suggested Launcher Policy

- Launcher inspects device capability.
- Launcher offers only roles the machine can realistically run.
- Launcher refuses a second instance of the same role on the same machine.
- Launcher should never collapse all roles into one runtime loop.
- If future server-farm miners are introduced, they should be a separate roadmap item, not the default model.
