# Inter-PC Git Bridge

Messages between Grouchly and Beastly via the shared repo.

Convention:
- `grouchly/` — messages from Grouchly to Beastly
- `beastly/` — messages from Beastly to Grouchly
- Each message: `<timestamp>-<topic>.md`
- Sender commits + pushes; receiver pulls + reads.
- Never include secrets, keys, or passwords in bridge messages.
