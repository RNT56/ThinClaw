//! Soul rendering/support crate.

pub mod soul;

pub use soul::{
    CANONICAL_SECTIONS, CANONICAL_SOUL_SCHEMA, CanonicalSoul, LOCAL_SECTIONS, LOCAL_SOUL_SCHEMA,
    LocalSoulOverlay, PackExpression, canonical_pack_name, canonical_schema_version,
    canonical_seed_pack, compose_seeded_soul, pack_asset_markdown, parse_canonical_soul,
    parse_local_soul_overlay, parse_pack_expression, render_canonical_prompt_block,
    render_canonical_soul, render_local_prompt_block, render_local_soul_overlay,
    summarize_canonical_soul, validate_canonical_soul, validate_local_overlay,
};
