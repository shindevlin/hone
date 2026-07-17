-- Verasens B2B dashboard schema. NEW service DB, deliberately separate from
-- chain state and from the consumer bot store (see PRD 1.4 DECISION RECORD).

CREATE TABLE org (
    id                  UUID PRIMARY KEY,
    name                TEXT NOT NULL,
    stripe_customer_id  TEXT,
    plan_id             TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE member (
    id              UUID PRIMARY KEY,
    org_id          UUID NOT NULL REFERENCES org(id) ON DELETE CASCADE,
    email           TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('owner','admin','analyst','billing')),
    status          TEXT NOT NULL CHECK (status IN ('pending_activation','active','suspended')),
    password_hash   TEXT NOT NULL,
    totp_secret     TEXT,
    totp_enabled    BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX member_email_lower_unique_idx ON member (lower(email));
CREATE INDEX member_org_id_idx ON member(org_id);

CREATE TABLE session (
    id                      UUID PRIMARY KEY,
    member_id               UUID NOT NULL REFERENCES member(id) ON DELETE CASCADE,
    org_id                  UUID NOT NULL REFERENCES org(id) ON DELETE CASCADE,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    absolute_expires_at     TIMESTAMPTZ NOT NULL,
    revoked_at              TIMESTAMPTZ
);

CREATE INDEX session_member_id_idx ON session(member_id);

CREATE TABLE api_key (
    id              UUID PRIMARY KEY,
    org_id          UUID NOT NULL REFERENCES org(id) ON DELETE CASCADE,
    prefix          TEXT NOT NULL UNIQUE,
    hash            TEXT NOT NULL,
    scopes          TEXT[] NOT NULL,
    created_by      UUID NOT NULL REFERENCES member(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at    TIMESTAMPTZ,
    revoked_at      TIMESTAMPTZ
);

CREATE INDEX api_key_org_id_idx ON api_key(org_id);

CREATE TABLE usage_event (
    id                      UUID PRIMARY KEY,
    org_id                  UUID NOT NULL REFERENCES org(id) ON DELETE CASCADE,
    api_key_id              UUID REFERENCES api_key(id),
    member_id               UUID REFERENCES member(id),
    query_hash              TEXT NOT NULL,
    units                   BIGINT NOT NULL,
    ts                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    stripe_meter_event_id   TEXT
);

CREATE INDEX usage_event_org_id_ts_idx ON usage_event(org_id, ts DESC);
CREATE INDEX usage_event_unreported_idx ON usage_event(org_id) WHERE stripe_meter_event_id IS NULL;

CREATE TABLE settlement_intent (
    id                  UUID PRIMARY KEY,
    usage_event_id      UUID NOT NULL REFERENCES usage_event(id) ON DELETE CASCADE,
    sensor_id           TEXT NOT NULL,
    owner_account       TEXT NOT NULL,
    batch_hash          TEXT,
    usd_amount_cents    BIGINT NOT NULL,
    dreams_amount       BIGINT,
    fx_rate_id          TEXT,
    status              TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','posting','posted','failed')),
    chain_entry_hash    TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    posted_at           TIMESTAMPTZ
);

CREATE INDEX settlement_intent_status_idx ON settlement_intent(status);
CREATE INDEX settlement_intent_usage_event_idx ON settlement_intent(usage_event_id);
