-- Direct Workbench asset registry.
-- Migration: 20260301000001_direct_assets
--
-- This table is intentionally separate from ThinClaw agent memory/workspace
-- state. It stores Desktop direct-chat/media assets that can later be exposed
-- through explicit handoff APIs.

CREATE TABLE IF NOT EXISTS direct_assets (
    id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL DEFAULT 'direct_workbench',
    kind TEXT NOT NULL,
    origin TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'ready',
    visibility TEXT NOT NULL DEFAULT 'private',
    path TEXT NOT NULL,
    mime_type TEXT,
    size_bytes INTEGER,
    sha256 TEXT,
    prompt TEXT,
    provider TEXT,
    style_id TEXT,
    aspect_ratio TEXT,
    resolution TEXT,
    width INTEGER,
    height INTEGER,
    seed INTEGER,
    thumbnail_path TEXT,
    is_favorite INTEGER NOT NULL DEFAULT 0,
    tags TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_direct_assets_namespace_kind
ON direct_assets(namespace, kind, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_direct_assets_provider
ON direct_assets(provider);

CREATE INDEX IF NOT EXISTS idx_direct_assets_favorite
ON direct_assets(is_favorite);

INSERT OR IGNORE INTO direct_assets (
    id, namespace, kind, origin, status, visibility, path,
    prompt, provider, style_id, aspect_ratio, resolution, width, height,
    seed, thumbnail_path, is_favorite, tags, created_at, updated_at
)
SELECT
    id, 'direct_workbench', 'generated_image', 'generated', 'ready', 'private', file_path,
    prompt, provider, style_id, aspect_ratio, resolution, width, height,
    seed, thumbnail_path, is_favorite, tags, created_at, created_at
FROM generated_images;
