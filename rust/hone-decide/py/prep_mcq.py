"""Public multiple-choice benchmarks -> hone-decide rows.

Each row: {id, task, split, request: {state, questions: {q: ...}}, label} where label is
the answer key ("A".."E" for choice, "true"/"false" for BoolQ noul).
Splits never use a benchmark's test split for training: when the public test split has
no labels, the validation split becomes our test and cal is carved from train.
  python prep_mcq.py --out mcq/ [--n-train 1000 --n-cal 200 --n-test 300]
"""
import argparse, json, random, zlib
from pathlib import Path
from datasets import load_dataset

L = "ABCDE"
LICENSES = {
    "arc_easy": "CC-BY-SA-4.0 (allenai/ai2_arc)", "arc_challenge": "CC-BY-SA-4.0 (allenai/ai2_arc)",
    "commonsense_qa": "MIT (tau/commonsense_qa)", "openbookqa": "Apache-2.0 (allenai/openbookqa)",
    "mmlu": "MIT (cais/mmlu)", "hellaswag": "MIT (Rowan/hellaswag)",
    "piqa": "AFL-3.0 (ybisk/piqa via parquet mirror)", "boolq": "CC-BY-SA-3.0 (google/boolq)",
}
MMLU_SUBJECTS = ["computer_security", "high_school_physics", "elementary_mathematics",
                 "high_school_macroeconomics", "international_law", "high_school_biology"]
MAX_CHARS = 1500  # keep prompts short enough for nebra (~500 tokens)


def choice(task, i, split, state, question, options, answer):
    crit = {L[k]: o for k, o in enumerate(options)}
    return {"id": f"{task}-{split}-{i:05d}", "task": task, "split": split, "label": L[answer],
            "request": {"state": state, "questions": {"q": {"type": "choice", "instructions": question, "criteria": crit}}}}


def arc(name):
    def conv(ex, i, split):
        labels = ex["choices"]["label"]
        if ex["answerKey"] not in labels:
            return None
        return choice(name, i, split, "", ex["question"], ex["choices"]["text"], labels.index(ex["answerKey"]))
    cfg = "ARC-Easy" if name == "arc_easy" else "ARC-Challenge"
    d = load_dataset("allenai/ai2_arc", cfg)
    return conv, {"train": d["train"], "cal": d["validation"], "test": d["test"]}


def csqa():
    def conv(ex, i, split):
        return choice("commonsense_qa", i, split, "", ex["question"], ex["choices"]["text"],
                      ex["choices"]["label"].index(ex["answerKey"]))
    d = load_dataset("tau/commonsense_qa")
    return conv, {"train": d["train"], "test": d["validation"]}


def obqa():
    def conv(ex, i, split):
        return choice("openbookqa", i, split, "", ex["question_stem"], ex["choices"]["text"],
                      ex["choices"]["label"].index(ex["answerKey"]))
    d = load_dataset("allenai/openbookqa", "main")
    return conv, {"train": d["train"], "cal": d["validation"], "test": d["test"]}


def mmlu():
    from datasets import concatenate_datasets
    def conv(ex, i, split):
        return choice("mmlu", i, split, f"Subject: {ex['subject'].replace('_', ' ')}", ex["question"], ex["choices"], ex["answer"])
    parts = [load_dataset("cais/mmlu", s) for s in MMLU_SUBJECTS]
    return conv, {"cal": concatenate_datasets([p["validation"] for p in parts]),
                  "test": concatenate_datasets([p["test"] for p in parts])}


def hellaswag():
    def conv(ex, i, split):
        if ex["label"] == "":
            return None
        return choice("hellaswag", i, split, f"{ex['activity_label']}: {ex['ctx']}",
                      "Which ending best continues the passage?", ex["endings"], int(ex["label"]))
    d = load_dataset("Rowan/hellaswag")
    return conv, {"train": d["train"], "test": d["validation"]}


def piqa():
    def conv(ex, i, split):
        return choice("piqa", i, split, f"Goal: {ex['goal']}", "Which solution achieves the goal?",
                      [ex["sol1"], ex["sol2"]], int(ex["label"]))
    for repo in ("baber/piqa", "ybisk/piqa"):
        try:
            d = load_dataset(repo)
            return conv, {"train": d["train"], "test": d["validation"]}
        except Exception as e:  # noqa: BLE001
            print("piqa", repo, "failed:", str(e)[:120])
    raise RuntimeError("no piqa source")


def boolq():
    def conv(ex, i, split):
        q = ex["question"].strip()
        q = q[0].upper() + q[1:] + ("" if q.endswith("?") else "?")
        return {"id": f"boolq-{split}-{i:05d}", "task": "boolq", "split": split,
                "label": "true" if ex["answer"] else "false",
                "request": {"state": ex["passage"], "questions": {"q": {"type": "noul", "instructions": q}}}}
    d = load_dataset("google/boolq")
    return conv, {"train": d["train"], "test": d["validation"]}


SOURCES = {"arc_easy": lambda: arc("arc_easy"), "arc_challenge": lambda: arc("arc_challenge"),
           "commonsense_qa": csqa, "openbookqa": obqa, "mmlu": mmlu, "hellaswag": hellaswag,
           "piqa": piqa, "boolq": boolq}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--out", required=True)
    p.add_argument("--n-train", type=int, default=1000)
    p.add_argument("--n-cal", type=int, default=200)
    p.add_argument("--n-test", type=int, default=300)
    a = p.parse_args()
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    files = {s: open(out / f"{s}.jsonl", "w") for s in ("train", "cal", "test")}
    meta = {}
    for name, src in SOURCES.items():
        conv, splits = src()
        rng = random.Random(zlib.crc32(name.encode()))
        if "cal" not in splits and "train" in splits:  # carve cal from the end of train
            tr = splits["train"]; k = min(len(tr) // 5, 2000)
            splits = {**splits, "train": tr.select(range(len(tr) - k)), "cal": tr.select(range(len(tr) - k, len(tr)))}
        counts = {}
        for split, n in (("train", a.n_train), ("cal", a.n_cal), ("test", a.n_test)):
            if split not in splits:
                counts[split] = 0
                continue
            ds = splits[split]
            idx = list(range(len(ds))); rng.shuffle(idx)
            rows = []
            for j in idx:
                r = conv(ds[j], len(rows), split)
                if r and len(json.dumps(r["request"], ensure_ascii=False)) <= MAX_CHARS:
                    rows.append(r)
                if len(rows) >= n:
                    break
            for r in rows:
                files[split].write(json.dumps(r, ensure_ascii=False) + "\n")
            counts[split] = len(rows)
        meta[name] = {"license": LICENSES[name], **counts}
        print(name, counts, flush=True)
    (out / "SOURCES.json").write_text(json.dumps(meta, indent=2))


if __name__ == "__main__":
    main()
