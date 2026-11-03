"""Fixed HTTP test batch: first N rows per task of a hone-decide jsonl -> /decide payloads.
  python make_http_batch.py mcq/test.jsonl batch.jsonl [N=25]"""
import json, sys
from collections import Counter
src, dst = sys.argv[1], sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 25
c = Counter()
with open(dst, "w") as f:
    for l in open(src):
        r = json.loads(l)
        if c[r["task"]] >= n:
            continue
        c[r["task"]] += 1
        q = r["request"]["questions"]["q"]
        body = {"context": r["request"]["state"], "question": q["instructions"], "kind": q["type"]}
        if q["type"] == "choice":
            body["candidates"] = q["criteria"]
        f.write(json.dumps({"id": r["id"], "task": r["task"], "label": r["label"], "body": body}, ensure_ascii=False) + "\n")
print(dict(c))
