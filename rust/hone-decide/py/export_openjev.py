"""Export an Open-Jev checkpoint for the Rust runtime (hone-decide, qwen35 arch).

Streams the pinned Qwen3.5 base shards, keeps only the text backbone
(model.language_model.*), merges the LoRA adapter in fp32
(W' = W + (alpha/r) * B @ A), and writes:
  out/model.safetensors   merged backbone, keys without the language_model prefix
  out/head.safetensors    head.weight [1,H], head.bias [1], temperature [1] (f32)
  out/config.json         the base text_config
  out/tokenizer.json      copied from the base
  out/decide.json         arch + prompt template for the Rust side
No vision tower, mtp or lm_head is exported (Open-Jev drops them too).
  python export_openjev.py --base BASE_DIR --checkpoint CKPT --out OUT [--dtype f32|bf16]
"""
import argparse, json, shutil
from pathlib import Path

import torch
from safetensors import safe_open
from safetensors.torch import load_file, save_file

PREFIX = "model.language_model."


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--base", required=True)
    p.add_argument("--checkpoint", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--dtype", default="f32", choices=["f32", "bf16"])
    a = p.parse_args()
    base, ck, out = Path(a.base), Path(a.checkpoint), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    acfg = json.loads((ck / "adapter" / "adapter_config.json").read_text())
    scale = acfg["lora_alpha"] / acfg["r"]
    lora = load_file(str(ck / "adapter" / "adapter_model.safetensors"))
    # peft keys: base_model.model.layers.N.<mod>.lora_A.weight
    pairs = {}
    for k, v in lora.items():
        name = k.replace("base_model.model.", "", 1)
        mod, which = name.rsplit(".lora_", 1)
        pairs.setdefault(mod, {})[which.split(".")[0]] = v.float()
    files = sorted(base.glob("*.safetensors"))
    tensors, merged = {}, 0
    dt = torch.float32 if a.dtype == "f32" else torch.bfloat16
    for f in files:
        with safe_open(str(f), framework="pt") as h:
            for k in h.keys():
                if not k.startswith(PREFIX):
                    continue
                name = k[len(PREFIX):]
                t = h.get_tensor(k).float()
                if name.endswith(".weight") and name[: -len(".weight")] in pairs:
                    ab = pairs.pop(name[: -len(".weight")])
                    t = t + scale * (ab["B"] @ ab["A"])
                    merged += 1
                tensors[name] = t.to(dt).contiguous()
    if pairs:
        raise SystemExit(f"unmerged LoRA modules: {sorted(pairs)[:5]}")
    save_file(tensors, str(out / "model.safetensors"))
    head = torch.load(ck / "head.pt", map_location="cpu", weights_only=True)
    temp = json.loads((ck / "temperature.json").read_text())["temperature"]
    save_file({"head.weight": head["weight"].float().contiguous(), "head.bias": head["bias"].float().contiguous(),
               "temperature": torch.tensor([temp], dtype=torch.float32)}, str(out / "head.safetensors"))
    cfg = json.loads((base / "config.json").read_text())
    (out / "config.json").write_text(json.dumps(cfg.get("text_config", cfg), indent=2))
    shutil.copy(base / "tokenizer.json", out / "tokenizer.json")
    mj = json.loads((ck / "model.json").read_text())
    (out / "decide.json").write_text(json.dumps({
        "arch": "qwen35", "name": f"open-jev-{cfg.get('text_config', cfg).get('hidden_size')}",
        "source": {"open_jev_checkpoint": str(ck), "base_model": mj["model_id"], "base_revision": mj["revision"]},
        "prompt_prefix": "<|im_start|>user\n",
        "prompt_suffix": "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n",
        "max_length": mj["max_length"], "weights": "model.safetensors", "head": "head.safetensors",
    }, indent=2))
    print(json.dumps({"tensors": len(tensors), "lora_merged": merged, "scale": scale, "temperature": temp}))


if __name__ == "__main__":
    main()
