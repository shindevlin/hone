"""Distil an Open-Jev teacher into a small student with the same head design.

Student = a small causal LM backbone (SmolLM2) + f32 scalar head on the final-normed
hidden state of each candidate prompt's last token, initialised from
E[Yes] - E[No] like Open-Jev. Trained on the teacher's calibrated soft
probabilities with KL(teacher || student); the temperature is then refit on the
held-out calibration split (Open-Jev's fit_temperature). Prompts come from
Open-Jev's own compile_request/candidate_prompts (PYTHONPATH=<Open-Jev>).

  python train_student.py --base SmolLM2-135M-Instruct --data data/ --teacher teacher/ --out out/
data/{train,cal,test}.jsonl from gen_tasks.py; teacher/{train,cal,test}.jsonl from jev_score.py.
"""
import argparse, json, math, random, shutil, time
from collections import defaultdict
from pathlib import Path

import torch
import torch.nn.functional as F
from safetensors.torch import save_file
from transformers import AutoModel, AutoTokenizer

from jev.api import candidate_prompts, compile_request
from jev.metrics import evaluate_probabilities, fit_temperature, softmax

PREFIX, SUFFIX = "<|im_start|>user\n", "<|im_end|>\n<|im_start|>assistant\n"


def load(data, teacher, split):
    """data/teacher may be comma-separated lists of dirs (e.g. HONE tasks + public MCQ)."""
    rows = []
    for dd, td in zip(data.split(","), teacher.split(",")):
        t = {}
        for l in open(Path(td) / f"{split}.jsonl"):
            r = json.loads(l)
            t[r["id"]] = r["probs"]
        for l in open(Path(dd) / f"{split}.jsonl"):
            r = json.loads(l)
            if r["id"] not in t:
                continue
            if "request" in r:
                rec = compile_request(r["request"]["state"], r["request"]["questions"])[0]
            else:
                rec = compile_request(r["context"], {"q": {"type": "choice", "instructions": r["question"], "criteria": r["candidates"]}})[0]
            rows.append({"id": r["id"], "task": r["task"], "kind": rec["kind"],
                         "prompts": [PREFIX + p + SUFFIX for p in candidate_prompts(rec)],
                         "teacher": t[r["id"]], "label": rec["answer_keys"].index(r["label"])})
    return rows


class Student(torch.nn.Module):
    def __init__(self, base):
        super().__init__()
        self.tok = AutoTokenizer.from_pretrained(base)
        self.tok.padding_side = "right"
        if self.tok.pad_token_id is None:
            self.tok.pad_token = self.tok.eos_token
        self.backbone = AutoModel.from_pretrained(base, dtype=torch.float32, attn_implementation="sdpa")
        emb = self.backbone.get_input_embeddings().weight
        yes, no = self.tok.encode("Yes", add_special_tokens=False), self.tok.encode("No", add_special_tokens=False)
        self.head = torch.nn.Linear(emb.shape[1], 1)
        with torch.no_grad():
            if len(yes) == 1 and len(no) == 1:
                self.head.weight.copy_((emb[yes[0]] - emb[no[0]]).float().unsqueeze(0))
            self.head.bias.zero_()

    def forward(self, prompts, max_len, amp=True):
        enc = self.tok(prompts, padding=True, return_tensors="pt", add_special_tokens=True)
        if enc["input_ids"].shape[1] > max_len:
            raise ValueError(f"prompt of {enc['input_ids'].shape[1]} tokens exceeds {max_len}")
        enc = {k: v.cuda() for k, v in enc.items()}
        with torch.autocast("cuda", dtype=torch.bfloat16, enabled=amp):
            h = self.backbone(**enc).last_hidden_state
        lens = enc["attention_mask"].sum(-1) - 1
        last = h[torch.arange(h.shape[0], device=h.device), lens]
        return self.head(last.float()).squeeze(-1)


def batched_logits(model, rows, bs, max_len, amp=True):
    flat = [p for r in rows for p in r["prompts"]]
    s = model(flat, max_len, amp)
    out, i = [], 0
    for r in rows:
        n = len(r["prompts"])
        v = s[i:i + n]
        if r.get("kind") == "noul":  # Open-Jev: noul logits are [0, s]
            v = torch.stack([torch.zeros_like(v[0]), v[0]])
        out.append(v)
        i += n
    return out


def evaluate(model, rows, bs, max_len, temperature=None):
    model.eval()
    logits = []
    with torch.no_grad():
        for i in range(0, len(rows), bs):
            # fp32 like the Rust runtime (bf16 autocast moved logits by up to ~1.1)
            logits += [x.float().cpu().tolist() for x in batched_logits(model, rows[i:i + bs], bs, max_len, amp=False)]
    model.train()
    return logits


def report(rows, logits, temperature):
    by = defaultdict(list)
    for r, lg in zip(rows, logits):
        by[r["task"]].append((r, softmax(lg, temperature)))
    by["ALL"] = [x for k in list(by) for x in by[k]]
    out = {}
    for task, items in by.items():
        sp = [p for _, p in items]
        tp = [r["teacher"] for r, _ in items]
        lab = [r["label"] for r, _ in items]
        argmax = lambda v: max(range(len(v)), key=v.__getitem__)
        s = evaluate_probabilities(lab, sp)
        t = evaluate_probabilities(lab, tp)
        out[task] = {"n": len(items),
                     "agreement": sum(argmax(a) == argmax(b) for a, b in zip(sp, tp)) / len(items),
                     "student_acc": s["accuracy"], "student_ece": s["multiclass_ece"], "student_nll": s["nll"],
                     "teacher_acc": t["accuracy"], "teacher_ece": t["multiclass_ece"],
                     "mean_kl": sum(sum(q * math.log(max(q, 1e-12) / max(p, 1e-12)) for q, p in zip(b, a))
                                    for a, b in zip(sp, tp)) / len(items)}
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--data", required=True)
    ap.add_argument("--teacher", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--epochs", type=float, default=3)
    ap.add_argument("--lr", type=float, default=1e-4)
    ap.add_argument("--head-lr", type=float, default=1e-3)
    ap.add_argument("--bs", type=int, default=4)
    ap.add_argument("--max-len", type=int, default=2048)
    ap.add_argument("--name", default="student")
    ap.add_argument("--init", help="warm start: a student dir (model.safetensors + head.safetensors)")
    ap.add_argument("--label-weight", type=float, default=0.0,
                    help="0 = pure distillation (KL to teacher); >0 mixes in CE on the task's ground-truth label")
    a = ap.parse_args()
    random.seed(0); torch.manual_seed(0)
    train, cal, test = (load(a.data, a.teacher, s) for s in ("train", "cal", "test"))
    print(json.dumps({"train": len(train), "cal": len(cal), "test": len(test)}), flush=True)
    model = Student(a.base)
    if a.init:
        from safetensors.torch import load_file
        missing, unexpected = model.backbone.load_state_dict(load_file(str(Path(a.init) / "model.safetensors")), strict=False)
        hd = load_file(str(Path(a.init) / "head.safetensors"))
        with torch.no_grad():
            model.head.weight.copy_(hd["head.weight"]); model.head.bias.copy_(hd["head.bias"])
        print(json.dumps({"init": a.init, "missing": len(missing), "unexpected": len(unexpected)}), flush=True)
    model = model.cuda()
    model.backbone.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
    model.train()
    import bitsandbytes as bnb
    opt = bnb.optim.AdamW8bit([{"params": model.backbone.parameters(), "lr": a.lr},
                               {"params": model.head.parameters(), "lr": a.head_lr}], weight_decay=0.0)
    steps = int(math.ceil(len(train) / a.bs) * a.epochs)
    sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: min(1.0, (s + 1) / 50) * max(0.0, 1 - s / steps))
    step, t0 = 0, time.time()
    while step < steps:
        random.shuffle(train)
        for i in range(0, len(train), a.bs):
            if step >= steps:
                break
            batch = train[i:i + a.bs]
            loss = 0.0
            for r, z in zip(batch, batched_logits(model, batch, a.bs, a.max_len)):
                q = torch.tensor(r["teacher"], device=z.device)
                lp = z.log_softmax(-1)
                loss = loss + (1 - a.label_weight) * F.kl_div(lp, q, reduction="sum") - a.label_weight * lp[r["label"]]
            loss = loss / len(batch)
            opt.zero_grad(set_to_none=True)
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step(); sched.step(); step += 1
            if step % 50 == 0:
                print(json.dumps({"step": step, "of": steps, "kl": round(loss.item(), 4),
                                  "s": round(time.time() - t0)}), flush=True)
    cal_logits = evaluate(model, cal, a.bs * 2, a.max_len)
    # recalibrate against the held-out ground truth (the teacher's own T was fit the same way)
    temperature = fit_temperature(cal_logits, [r["label"] for r in cal])
    test_logits = evaluate(model, test, a.bs * 2, a.max_len)
    rep = {"temperature": temperature, "test": report(test, test_logits, temperature),
           "test_T1": report(test, test_logits, 1.0)["ALL"]}
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    (out / "eval.json").write_text(json.dumps(rep, indent=2))
    with open(out / "test_preds.jsonl", "w") as f:
        for r, lg in zip(test, test_logits):
            f.write(json.dumps({"id": r["id"], "logits": lg, "probs": softmax(lg, temperature)}) + "\n")
    sd = {k: v.float().contiguous() for k, v in model.backbone.state_dict().items() if "rotary" not in k}
    save_file(sd, str(out / "model.safetensors"))
    save_file({"head.weight": model.head.weight.detach().float().cpu().contiguous(),
               "head.bias": model.head.bias.detach().float().cpu().contiguous(),
               "temperature": torch.tensor([temperature], dtype=torch.float32)}, str(out / "head.safetensors"))
    shutil.copy(Path(a.base) / "config.json", out / "config.json")
    shutil.copy(Path(a.base) / "tokenizer.json", out / "tokenizer.json")
    (out / "decide.json").write_text(json.dumps({
        "arch": "llama", "name": a.name, "weights": "model.safetensors", "head": "head.safetensors",
        "prompt_prefix": PREFIX, "prompt_suffix": SUFFIX, "max_length": a.max_len,
        "distilled_from": str(a.teacher), "label_weight": a.label_weight, "base": str(a.base), "init": a.init}, indent=2))
    print(json.dumps(rep["test"]["ALL"]), flush=True)


if __name__ == "__main__":
    main()
