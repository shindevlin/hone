# Who may merge two bird ids on chain, and on what evidence (proposal)

`bird_cards::merge` (branch seats-cards) already fixes the outcome: the earliest name wins and both
cards stay, with a `merged_into` pointer. This file sets out who may trigger a merge and what
evidence it needs. No merge entry exists yet. Proposed name: `BIRD_MERGE {from, into, evidence}`.

## Rules

1. **Same place, same species.** `from` and `into` must have the same species and res-7 cell (both are
   on chain from BIRD_OBSERVE/claim). There are no cross-cell merges. A bird that moves is a new id.
2. **Who signs.** The merge must be signed by a backed seat whose key reported BOTH ids. Reported
   means the seat filed BIRD_OBSERVE for the id, or filed SensorSummary reports that list the id
   in the voiceprint summary on >= 3 nights. A seat that never heard the bird cannot merge it.
3. **Kind of evidence**, one of:
   - `auto`: the seat's own crate emitted `Event::Merged` (centroids >= merge_tau and earliest-name
     order). Evidence = `{nights_overlap, centroid_cos_q8, from_nights, into_nights}` (summary stats only,
     never voiceprints, which stay on the device).
   - `owner`: the device owner linked them by hand (`Voiceprinter::link`). This is valid only if the
     signer's owner account is `named_by` of at least one of the two cards, or both are unnamed.
4. **Consent of the other namer.** If both ids are named by different accounts, the merge is held
   as pending for 2,880 epochs. It lands only if (a) the other namer co-signs, or (b) a second
   backed seat, owned by a different account and itself satisfying rule 2, files the same merge. If
   neither happens it lapses. This stops one seat from absorbing someone else's named bird to take
   its card's history.
5. **Rate and cost.** At most 4 merges per seat per 120 epochs. The fee is paid by the seat owner.
   A merge that is later reversed (rule 6) counts as a bad report for slashing.
6. **Reversal.** A split filed by any seat satisfying rule 2 with `auto` evidence (`Event::Split`
   on the merged id) re-opens the absorbed card. Names and cards are never deleted, only pointed.
7. **Gate.** BIRD_MERGE is refused for species whose `naming_allowed` is false in the shipped
   gate table hash, the same gate as naming.

## What the crate already gives the integration

- `Event::Merged{from, into, dropped_name}` from automatic maintenance, and `link(a, b)` from the user.
  The survivor follows the same order as `bird_cards::merge`.
- `resolve(id)` follows merges locally, so the app keeps working while a chain merge is pending.
