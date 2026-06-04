-- Generated images metadata
CREATE TABLE IF NOT EXISTS generated_images (
    id TEXT PRIMARY KEY NOT NULL,
    prompt TEXT NOT NULL,
    style_id TEXT,
    provider TEXT NOT NULL DEFAULT 'local',
    aspect_ratio TEXT NOT NULL DEFAULT '1:1',
    resolution TEXT,
    width INTEGER,
    height INTEGER,
    seed INTEGER,
    file_path TEXT NOT NULL,
    thumbnail_path TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    is_favorite INTEGER NOT NULL DEFAULT 0,
    tags TEXT
);

-- Index for common queries
CREATE INDEX IF NOT EXISTS idx_generated_images_created_at ON generated_images(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_generated_images_provider ON generated_images(provider);
CREATE INDEX IF NOT EXISTS idx_generated_images_is_favorite ON generated_images(is_favorite);
