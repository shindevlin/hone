# Quantise a student and score f32 / Q8_0 / Q4 on test500.jsonl with the Rust runtime.
#   bash qeval.sh <student-name>   (in ~/decide on grouchly: students/<name>, test_full.jsonl, test500.jsonl)
cd ~/decide; R=~/hone-decide-wt/rust/target/release/hone-decide; S=students/$1
$R quantize --model $S --out $S-q8 --dtype q8_0; $R quantize --model $S --out $S-q4 --dtype q4
for v in "" -q8 -q4; do RAYON_NUM_THREADS=8 $R score --model $S$v --rows test500.jsonl --out qe_$1$v.jsonl; echo "$1$v $(python3 py/task_eval.py test_full.jsonl qe_$1$v.jsonl qe_$1.jsonl)"; done
du -sh $S-q8/*.gguf $S-q4/*.gguf
