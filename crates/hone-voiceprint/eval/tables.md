| set | clips | birds | enrol -> test | pipeline | AUC | EER | k=1 P / R | k=5 P / R (held-out tau) |
|---|---|---|---|---|---|---|---|---|
| littleowl | 952 | 16 | acrossyear-trn -> acrossyear-tst | raw | 0.725 | 0.337 | 0.42 / 0.30 | 0.47 / 0.44 (tau 0.92) |
| littleowl | 952 | 16 | acrossyear-trn -> acrossyear-tst | c-raw | 0.846 | 0.232 | 0.85 / 0.20 | 0.90 / 0.56 (tau 0.46) |
| littleowl | 952 | 16 | acrossyear-trn -> acrossyear-tst | c-rp64 | 0.817 | 0.258 | 0.91 / 0.13 | 0.94 / 0.37 (tau 0.54) |
| pipit | 722 | 10 | acrossyear-trn -> acrossyear-tst | raw | 0.585 | 0.440 | 1.00 / 0.00 | 1.00 / 0.05 (tau 0.92) |
| pipit | 722 | 10 | acrossyear-trn -> acrossyear-tst | c-raw | 0.603 | 0.429 | 1.00 / 0.01 | 0.96 / 0.07 (tau 0.48) |
| pipit | 722 | 10 | acrossyear-trn -> acrossyear-tst | c-rp64 | 0.600 | 0.434 | 1.00 / 0.02 | 0.96 / 0.09 (tau 0.54) |
| chiffchaff_day | 2041 | 13 | withinyear-trn -> withinyear-tst | raw | 0.630 | 0.406 | 0.87 / 0.07 | 0.95 / 0.41 (tau 0.92) |
| chiffchaff_day | 2041 | 13 | withinyear-trn -> withinyear-tst | c-raw | 0.701 | 0.353 | 1.00 / 0.05 | 0.96 / 0.31 (tau 0.48) |
| chiffchaff_day | 2041 | 13 | withinyear-trn -> withinyear-tst | c-rp64 | 0.648 | 0.398 | 0.91 / 0.06 | 0.97 / 0.24 (tau 0.54) |
| chiffchaff_year | 524 | 10 | acrossyear-trn -> acrossyear-tst | raw | 0.496 | 0.504 | 1.00 / 0.00 | 0.00 / 0.00 (tau 0.92) |
| chiffchaff_year | 524 | 10 | acrossyear-trn -> acrossyear-tst | c-raw | 0.547 | 0.462 | 1.00 / 0.00 | 0.00 / 0.00 (tau 0.48) |
| chiffchaff_year | 524 | 10 | acrossyear-trn -> acrossyear-tst | c-rp64 | 0.530 | 0.476 | 0.00 / 0.00 | 0.00 / 0.00 (tau 0.54) |
| greattit_days | 1100 | 23 | early -> late | raw | 0.689 | 0.373 | 1.00 / 0.00 | 0.99 / 0.12 (tau 0.92) |
| greattit_days | 1100 | 23 | early -> late | c-raw | 0.731 | 0.332 | 1.00 / 0.03 | 0.99 / 0.31 (tau 0.48) |
| greattit_days | 1100 | 23 | early -> late | c-rp64 | 0.686 | 0.375 | 0.95 / 0.05 | 0.92 / 0.23 (tau 0.54) |
| kiskadee_halves | 464 | 46 | A -> B | raw | 0.984 | 0.064 | 1.00 / 0.07 | 1.00 / 0.26 (tau 0.92) |
| kiskadee_halves | 464 | 46 | A -> B | c-raw | 0.993 | 0.030 | 0.96 / 0.53 | 0.98 / 1.00 (tau 0.48) |
| kiskadee_halves | 464 | 46 | A -> B | c-rp64 | 0.974 | 0.079 | 0.96 / 0.55 | 0.98 / 0.96 (tau 0.54) |
| kiwi_seasons | 142 | 7 | early -> late | raw | 0.910 | 0.190 | 0.77 / 0.46 | 0.66 / 0.51 (tau 0.92) |
| kiwi_seasons | 142 | 7 | early -> late | c-raw | 0.892 | 0.193 | 0.68 / 0.77 | 0.57 / 0.90 (tau 0.48) |
| kiwi_seasons | 142 | 7 | early -> late | c-rp64 | 0.881 | 0.200 | 0.67 / 0.77 | 0.58 / 0.89 (tau 0.54) |

| species (replay) | back events | back precision | false back | back recall | call precision | birds -> with id | ids per bird | mean likelihood right / wrong |
|---|---|---|---|---|---|---|---|---|
| Athene_noctua | 74 | 1.0 | 0.0 | 0.201 | 0.999 | 135 -> 129 | 1.72 | 0.937 / 0.0 |
| Anthus_trivialis | 24 | 0.833 | 0.0 | 0.104 | 0.837 | 139 -> 84 | 1.64 | 0.955 / 0.985 |
| Phylloscopus_collybita@year | 0 | None | None | 0.0 | 1.0 | 146 -> 43 | 1.12 | 0.0 / 0.0 |
| Phylloscopus_collybita@day | 124 | 0.968 | 0.0 | 0.454 | 0.927 | 143 -> 108 | 1.45 | 0.965 / 0.988 |
| Parus_major | 5 | 1.0 | 0.0 | 0.059 | 1.0 | 137 -> 12 | 1.08 | 0.989 / 0.0 |
| Apteryx_haastii | 996 | 0.821 | 0.169 | 0.434 | 0.796 | 141 -> 133 | 3.05 | 0.907 / 0.878 |
