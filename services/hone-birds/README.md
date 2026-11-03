# hone-birds

Off-chain indexer for public bird sightings and user photos (v1). Runs on ferryvps:
`/opt/hone-birds/hone_birds.py`, systemd unit `hone-birds` (DynamicUser, 127.0.0.1:9460),
data in `/var/lib/hone-birds`. Caddy routes `https://honemesh.net/api/birds/*` to it.

Privacy: the phone publishes ~5 km cells inside the user's home area and GPS-precise cells
elsewhere. Sightings are anonymous with one-time ids unless the user opts in to show their
name, in which case they are signed with the phone's Ed25519 device key. Photos are
re-encoded on the phone (no EXIF) and published under UFL after the user agrees.

Next: anchor a daily hash of new sightings on chain; on-chain entry type later.
