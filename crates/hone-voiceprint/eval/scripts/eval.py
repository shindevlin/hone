"""Voiceprint evaluation: across-session verification and open-set re-identification.

usage: eval.py OUT.json SPECIES=npz:enrol_session:test_session [...]

For each species, the enrol session plays "earlier nights" and the test session "later nights".
Pipelines (all end in L2 norm + int8):
  raw      : Perch 1536-d, L2 normalised
  c-raw    : minus the population mean (estimated from all of that species' clips, unlabelled)
  c-rp64   : centred, then a fixed seeded Gaussian random projection to 64 dims
  c-pca64  : centred, then PCA-64 fitted on the OTHER species only (leave-species-out)
Metrics:
  verification AUC / EER on cross-session pairs (one clip each)
  open-set re-ID, k = clips aggregated per decision (a "night" of k calls from one bird):
    enrol half the individuals (centroid of their enrol-session voiceprints), 20 rotations;
    a decision names the best centroid if cosine >= tau else "unknown".
    precision = right names / all names given; recall = right names / decisions about enrolled birds.
  tau is chosen leave-species-out: the smallest tau where the OTHER species reach precision >= 0.9.
"""
import sys, json, numpy as np
from sklearn.metrics import roc_auc_score, roc_curve

OUT = sys.argv[1]
specs = {}
for a in sys.argv[2:]:
    sp, rest = a.split("=")
    npz, en, te = rest.split(":")
    specs[sp] = (npz, en, te)

def l2(x): return x / (np.linalg.norm(x, axis=-1, keepdims=True) + 1e-9)
def q8(x):
    x = l2(x); m = np.abs(x).max(axis=-1, keepdims=True) + 1e-9
    return np.round(x / m * 127).astype(np.int8)

def rp(seed, d, n):
    # Must match hone-voiceprint Projection::seeded (splitmix64 + Box-Muller, row-major d x n).
    s = np.uint64(seed); out = np.empty(d * n, np.float64)
    M = 0xFFFFFFFFFFFFFFFF
    st = seed & M
    def nxt():
        nonlocal st
        st = (st + 0x9E3779B97F4A7C15) & M
        z = st
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M
        return z ^ (z >> 31)
    i = 0
    while i < d * n:
        u1 = ((nxt() >> 11) + 1) / 9007199254740993.0
        u2 = (nxt() >> 11) / 9007199254740992.0
        r = np.sqrt(-2 * np.log(u1))
        out[i] = r * np.cos(2 * np.pi * u2); i += 1
        if i < d * n:
            out[i] = r * np.sin(2 * np.pi * u2); i += 1
    return out.reshape(d, n).astype(np.float32)

data = {}
for sp, (npz, en, te) in specs.items():
    z = dict(np.load(npz))  # decompress once
    m = np.isin(z["session"], [en, te])
    # aggregate frames of one clip (long clips give several frames) by mean
    clips = np.unique(z["clip"][m])
    E, L, S = [], [], []
    zc, ze, zl, zs = z["clip"][m], z["emb"][m], z["label"][m], z["session"][m]
    order = np.argsort(zc, kind="stable"); zc, ze, zl, zs = zc[order], ze[order], zl[order], zs[order]
    starts = np.searchsorted(zc, clips); ends = np.searchsorted(zc, clips, side="right")
    for a, b in zip(starts, ends):
        E.append(l2(ze[a:b]).mean(0)); L.append(zl[a]); S.append(zs[a])
    data[sp] = dict(E=l2(np.array(E)), L=np.array(L), S=np.array(S), en=en, te=te)

W64 = rp(0x484F4E45, 64, 1536)  # "HONE"
W128 = rp(0x484F4E45, 128, 1536)

def pipelines(sp):
    d = data[sp]; X = d["E"]; mu = X.mean(0)
    others = [o for o in data if o != sp]
    out = {"raw": q8(X), "c-raw": q8(X - mu), "c-rp64": q8((X - mu) @ W64.T),
           "c-rp128": q8((X - mu) @ W128.T)}
    Y = (X - mu) @ W64.T
    for lam in (0.1, 0.3):
        C = np.cov(Y.T); C = C + lam * np.trace(C) / C.shape[0] * np.eye(C.shape[0])
        ev, U = np.linalg.eigh(C); Wh = U @ np.diag(ev ** -0.5) @ U.T
        out[f"c-rp64-w{lam}"] = q8(Y @ Wh)
    if others:
        Y = np.concatenate([data[o]["E"] - data[o]["E"].mean(0) for o in others])
        _, _, vt = np.linalg.svd(Y - Y.mean(0), full_matrices=False)
        out["c-pca64"] = q8((X - mu) @ vt[:64].T)
        # LDA on other species' individual labels (each species centred), shrinkage, <=32 dims
        from sklearn.discriminant_analysis import LinearDiscriminantAnalysis as LDA
        Ys = np.concatenate([(data[o]["E"] - data[o]["E"].mean(0)) @ W64.T for o in others])
        Ls = np.concatenate([np.char.add(o + ":", data[o]["L"].astype(str)) for o in others])
        nc = min(32, len(np.unique(Ls)) - 1)
        lda = LDA(solver="eigen", shrinkage=0.3).fit(Ys, Ls)
        out["c-rp64-lda"] = q8(((X - mu) @ W64.T) @ lda.scalings_[:, :nc])
    return out

def cos(a, b): a = l2(a.astype(np.float32)); b = l2(b.astype(np.float32)); return a @ b.T

def verif(V, L, S, en, te):
    A, B = V[S == en], V[S == te]
    sims = cos(A, B); same = (L[S == en][:, None] == L[S == te][None, :])
    y, s = same.ravel(), sims.ravel()
    auc = roc_auc_score(y, s); fpr, tpr, thr = roc_curve(y, s)
    i = np.argmin(np.abs(fpr - (1 - tpr)))
    # calibration for likelihoods: logistic fit p(same | sim)
    from sklearn.linear_model import LogisticRegression
    lr = LogisticRegression(class_weight="balanced").fit(s[:, None], y)
    return dict(auc=round(float(auc), 4), eer=round(float(fpr[i]), 4),
                logit_a=float(lr.coef_[0][0]), logit_b=float(-lr.intercept_[0] / lr.coef_[0][0]))

def reid(V, L, S, en, te, k, taus, rot=20, seed=7):
    rng = np.random.default_rng(seed)
    ids = np.unique(L[S == en]); ids = np.array([i for i in ids if np.any((L == i) & (S == te))])
    best, right, enrolled = [], [], []
    Vn = l2(V.astype(np.float32))
    for r in range(rot):
        enr = rng.choice(ids, size=max(2, len(ids) // 2), replace=False)
        C = l2(np.array([Vn[(L == i) & (S == en)].mean(0) for i in enr]))
        for i in ids:
            T = np.where((L == i) & (S == te))[0]
            rng.shuffle(T)
            for j in range(0, len(T) - k + 1, k):
                q = l2(Vn[T[j:j + k]].mean(0))
                s = C @ q; b = int(np.argmax(s))
                best.append(s[b]); right.append(enr[b] == i); enrolled.append(i in enr)
    best, right, enrolled = map(np.array, (best, right, enrolled))
    out = {}
    for t in taus:
        named = best >= t; tp = int((named & right).sum()); n = int(named.sum()); e = int(enrolled.sum())
        out[t] = (tp / n if n else 1.0, tp / e if e else 0.0, n)
    return out

taus = [round(x, 2) for x in np.arange(0.0, 0.96, 0.02)]
R = {}
import pickle, os, hashlib
for sp in data:
    d = data[sp]
    ck = "/home/beastly/voiceprint-eval/out/cache-" + hashlib.sha1((sp + str(specs[sp]) + ",".join(sorted(data))).encode()).hexdigest()[:12] + ".pkl"
    if os.path.exists(ck):
        R[sp] = pickle.load(open(ck, "rb")); continue
    R[sp] = {"n_clips": int(len(d["L"])), "n_ind": int(len(np.unique(d["L"]))),
                           "enrol": d["en"], "test": d["te"], "pipelines": {}}
    for name, V in pipelines(sp).items():
        e = verif(V, d["L"], d["S"], d["en"], d["te"])
        e["reid"] = {k: reid(V, d["L"], d["S"], d["en"], d["te"], k, taus) for k in (1, 3, 5)}
        R[sp]["pipelines"][name] = e
    pickle.dump(R[sp], open(ck, "wb"))

# per-species oracle tau: smallest tau where that species alone reaches precision >= 0.9
for sp in R:
    for name in R[sp]["pipelines"]:
        for k in (1, 3, 5):
            best = None
            for t in taus:
                p, rc, n = R[sp]["pipelines"][name]["reid"][k][t]
                if p >= 0.9 and n > 0: best = (t, round(p, 3), round(rc, 3)); break
            R[sp]["pipelines"][name].setdefault("oracle", {})[k] = best
# leave-species-out tau: the strictest oracle tau among the OTHER species that have one
for sp in R:
    for name in R[sp]["pipelines"]:
        for k in (1, 3, 5):
            ts = [R[o]["pipelines"][name]["oracle"][k][0] for o in R if o != sp and name in R[o]["pipelines"] and R[o]["pipelines"][name]["oracle"][k]]
            tau = max(ts) if ts else taus[-1]
            p, rc, n = R[sp]["pipelines"][name]["reid"][k][tau]
            R[sp]["pipelines"][name].setdefault("op", {})[k] = dict(tau=float(tau), precision=round(p, 3), recall=round(rc, 3), named=n)
for sp in R:
    for name in R[sp]["pipelines"]:
        R[sp]["pipelines"][name]["reid"] = {k: {str(t): v for t, v in R[sp]["pipelines"][name]["reid"][k].items()} for k in (1, 3, 5)}
json.dump(R, open(OUT, "w"), indent=1, default=float)
for sp in R:
    print(f"== {sp}  clips={R[sp]['n_clips']} individuals={R[sp]['n_ind']}  {R[sp]['enrol']} -> {R[sp]['test']}")
    for name, e in R[sp]["pipelines"].items():
        ops = "  ".join(f"k{k}: tau={o['tau']} P={o['precision']} R={o['recall']}" for k, o in e["op"].items())
        orc = "  ".join(f"k{k}:{e['oracle'][k]}" for k in (1, 3, 5))
        print(f"  {name:8s} AUC={e['auc']} EER={e['eer']}  LOSO {ops}\n           oracle {orc}")
