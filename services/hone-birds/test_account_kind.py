"""Account kind at sign-up: personal vs business, business profile, free partner plan grants."""
import os, subprocess, sys, tempfile, time, urllib.request

from nacl.signing import SigningKey

import test_accounts as T


def main():
    data = tempfile.mkdtemp(prefix="hb-kind-")
    env = dict(os.environ, HONE_BIRDS_DATA=data, HONE_BIRDS_PORT=str(T.PORT), HONE_DEVICES_ADMIN_TOKEN=T.ADMIN,
               HONE_NODE_URL="http://127.0.0.1:1", HONE_SEALS="0")
    env.pop("HONE_MAIL_WEBHOOK", None)
    srv = subprocess.Popen([sys.executable, os.path.join(T.HERE, "hone_birds.py")], env=env, stderr=subprocess.PIPE)
    adm = {"X-Hone-Admin": T.ADMIN}
    try:
        for _ in range(50):
            try: urllib.request.urlopen(T.BASE + "/api/birds/health", timeout=1); break
            except Exception: time.sleep(0.2)
        def base(email):
            return {"name": "Joe", "email": email, "password": "correct horse battery", "ref": "none",
                    "consent": True, "consent_text": "pool", "seat": T.seat_proof(SigningKey.generate(), email)}
        # grant before sign-up: needs admin, known plan
        c, r = T.call("POST", "/api/accounts/admin/plan", {"email": "joe@example.org", "plan": "free_partner"}); assert c in (401, 403), r
        c, r = T.call("POST", "/api/accounts/admin/plan", {"email": "joe@example.org", "plan": "gold"}, adm); assert c == 400, r
        c, r = T.call("POST", "/api/accounts/admin/plan", {"email": "Joe@Example.org", "plan": "free_partner",
                      "note": "test partner: restaurant and eco site, always free"}, adm)
        assert c == 200 and r["applied"] is False, r
        # business needs a name; bad kind refused
        c, r = T.call("POST", "/api/accounts/signup", dict(base("joe@example.org"), kind="shop")); assert c == 400, r
        c, r = T.call("POST", "/api/accounts/signup", dict(base("joe@example.org"), kind="business", business={"name": " "})); assert c == 400 and "business name" in r["error"], r
        c, r = T.call("POST", "/api/accounts/signup", dict(base("joe@example.org"), kind="business",
                      business={"name": "Joe's <b>Falls</b> Cafe", "places": ["restaurant", "eco", "casino", "farm", "shop"]}))
        assert c == 200, r
        u = r["user"]
        assert u["kind"] == "business" and u["business"]["name"] == "Joe's bFalls/b Cafe", u
        assert u["business"]["places"] == ["restaurant", "eco", "farm"], u
        assert u["plan"] == "free_partner" and "always free" in u["plan_note"], u
        # personal sign-up: no business, no plan
        c, r = T.call("POST", "/api/accounts/signup", dict(base("ana@example.org"), kind="personal"))
        assert c == 200 and r["user"]["kind"] == "personal" and r["user"]["business"] is None and r["user"]["plan"] == "", r
        A = {"Authorization": "Bearer " + r["token"]}
        # older client: no kind sent -> null (not asked yet); can choose later through PUT /me
        c, r = T.call("POST", "/api/accounts/signup", base("old@example.org")); assert c == 200 and r["user"]["kind"] is None, r
        B = {"Authorization": "Bearer " + r["token"]}
        c, r = T.call("PUT", "/api/accounts/me", {"kind": "business", "business": {"name": "Old Farm", "places": ["farm"]}}, B); assert c == 200, r
        c, r = T.call("GET", "/api/accounts/me", headers=B); assert r["kind"] == "business" and r["business"]["name"] == "Old Farm", r
        # grant after sign-up applies at once
        c, r = T.call("POST", "/api/accounts/admin/plan", {"email": "ana@example.org", "plan": "free_partner"}, adm); assert r["applied"] is True, r
        c, r = T.call("GET", "/api/accounts/me", headers=A); assert r["plan"] == "free_partner", r
        print("ALL ACCOUNT KIND TESTS PASSED")
    finally:
        srv.terminate()
        err = srv.stderr.read().decode()
        assert "joe@example.org" not in err.lower(), "email in server log"


if __name__ == "__main__":
    main()
