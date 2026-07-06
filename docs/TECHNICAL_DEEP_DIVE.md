# BTCPC Technical Deep Dive

This page is for readers who want the mechanics, not just the product story.

If you are looking for the shortest way to get BTCPC running, start with
[`START_HERE`](START_HERE.md).

## What BTCPC Pays For

BTCPC turns real work into the work source for the chain:

- AI inference
- storage
- sensors
- service hosting
- clock timing

Those mechanisms are the chain's proof of compute. They are not side features.

## The Main Mechanisms

### Start flow

The public onboarding flow lives at [`/start`](https://honemesh.net/start) and is
described by [`/start.json`](https://honemesh.net/start.json). The manifest is the
machine-readable source of truth for the onboarding sequence.

### Username availability

The first step now asks for the on-chain username and checks whether it is
available before account creation. That avoids wasting setup time on a name
that already exists on chain.

### Account control

BTCPC supports:

- native BTCPC active-key sending
- controller mode, where an opted-in external chain controls spending
- secondary approval, which stays a separate outside-wallet factor

The controller path is policy-driven. It should be enforced by the protocol,
not by the UI alone.

### Shared start-state logic

The browser app, setup page, and start wizard all read the same manifest-driven
state helper. That keeps the public flow consistent and reduces duplicated logic.

## How to Improve BTCPC

If you find a cleaner, safer, or more deterministic way to do any of the above,
please contribute it back to the repository.

Good contribution targets:

- better onboarding clarity
- clearer chain-mechanism docs
- stricter deterministic agent flows
- better mobile approval UX
- safer controller and secondary-approval flows

If your change improves the protocol, documentation, or UX, open a pull request.
If it makes the chain easier to use without weakening security, it is probably
worth discussing.

## Reading Order

Recommended order:

1. [`START_HERE`](START_HERE.md)
2. [`HONE_WHITEPAPER`](HONE_WHITEPAPER.md)
3. [`ROADMAP`](ROADMAP.md)
4. [`contributing`](contributing.md)
