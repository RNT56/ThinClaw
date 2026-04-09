-- Identity registry for household-shared multi-actor support.
--
-- Stores principal-scoped actors and their linked channel endpoints so the
-- ingress layer can distinguish family members without splitting shared state.

CREATE TABLE actors (
    actor_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',

    preferred_delivery_channel TEXT,
    preferred_delivery_external_user_id TEXT,
    last_active_direct_channel TEXT,
    last_active_direct_external_user_id TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CHECK (
        (preferred_delivery_channel IS NULL) = (preferred_delivery_external_user_id IS NULL)
    ),
    CHECK (
        (last_active_direct_channel IS NULL) = (last_active_direct_external_user_id IS NULL)
    )
);

CREATE INDEX idx_actors_principal ON actors(principal_id);
CREATE INDEX idx_actors_status ON actors(status);

CREATE TABLE actor_endpoints (
    channel TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    actor_id UUID NOT NULL REFERENCES actors(actor_id) ON DELETE CASCADE,
    endpoint_metadata JSONB NOT NULL DEFAULT '{}',
    approval_status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel, external_user_id)
);

CREATE INDEX idx_actor_endpoints_actor_id ON actor_endpoints(actor_id);
CREATE INDEX idx_actor_endpoints_approval_status ON actor_endpoints(approval_status);
