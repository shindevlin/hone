"""Score decision rows with the real Open-Jev Python stack (reference + teacher).

Rows (jsonl): {id, context, question, candidates: {name: description|null}} or
{id, request: <Open-Jev request with state/questions>}.
Writes jsonl: {id, qid, kind, answer_keys, logits, probs, temperature, input_tokens}.

  PYTHONPATH=<Open-Jev checkout> python jev_score.py --checkpoint CKPT --rows rows.jsonl --out out.jsonl
Env: JEV_LOAD_4BIT=1 / JEV_LOAD_8BIT=1 / JEV_TORCH_DTYPE=float32 are honoured by Open-Jev itself.
--dump-prompts also writes each rendered candidate prompt and its token ids (tokenizer parity).
"""
import argparse, json, math, time
from pathlib import Path

import torch
from jev.api import compile_request, candidate_prompts
from jev.metrics import softmax
from jev.model import DecisionModel


def to_request(row):
    if "request" in row:
        return row["request"]
    return {"state": row["context"],
            "questions": {"q": {"type": "choice", "instructions": row["question"], "criteria": row["candidates"]}}}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--checkpoint", required=True)
    p.add_argument("--rows", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--device", default="cuda:0")
    p.add_argument("--batch-records", type=int, default=4)
    p.add_argument("--limit", type=int, default=0)
    p.add_argument("--dump-prompts")
    a = p.parse_args()
    ck = Path(a.checkpoint)
    temperature = json.loads((ck / "temperature.json").read_text())["temperature"]
    model = DecisionModel.load(ck, device=a.device)
    tok = model.tokenizer
    rows = [json.loads(l) for l in open(a.rows) if l.strip()]
    if a.limit:
        rows = rows[: a.limit]
    done = set()
    outp = Path(a.out)
    if outp.exists():
        done = {json.loads(l)["id"] for l in open(outp) if l.strip()}
    rows = [r for r in rows if r["id"] not in done]
    fo = open(outp, "a")
    fd = open(a.dump_prompts, "a") if a.dump_prompts else None
    t0, n = time.time(), 0
    pending = []

    def flush():
        nonlocal n
        if not pending:
            return
        records = [rec for _, rec in pending]
        with torch.inference_mode():
            logits = model(records)
        for (rid, rec), row in zip(pending, logits):
            lg = row.float().cpu().tolist()
            pr = softmax(lg, temperature=temperature)
            fo.write(json.dumps({"id": rid, "qid": rec["id"], "kind": rec["kind"], "answer_keys": rec["answer_keys"],
                                 "logits": lg, "probs": pr, "temperature": temperature}) + "\n")
            if fd:
                for prompt in candidate_prompts(rec):
                    text = tok.apply_chat_template([{"role": "user", "content": prompt}], tokenize=False,
                                                   add_generation_prompt=True, enable_thinking=False)
                    fd.write(json.dumps({"id": rid, "qid": rec["id"], "text": text,
                                         "ids": tok(text)["input_ids"]}, ensure_ascii=False) + "\n")
            n += 1
        fo.flush()
        pending.clear()

    for row in rows:
        req = to_request(row)
        for rec in compile_request(req["state"], req["questions"]):
            pending.append((row["id"] if rec["id"] == "q" else f'{row["id"]}/{rec["id"]}', rec))
            if len(pending) >= a.batch_records:
                flush()
                if n % 200 < a.batch_records:
                    print(json.dumps({"done": n, "rate_per_s": round(n / (time.time() - t0), 2)}), flush=True)
    flush()
    print(json.dumps({"done": n, "seconds": round(time.time() - t0, 1)}), flush=True)


if __name__ == "__main__":
    main()
