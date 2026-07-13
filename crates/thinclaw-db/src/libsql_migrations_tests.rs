use super::{DATA_REPAIRS, SCHEMA, UPGRADES};

#[test]
fn schema_includes_learning_tables() {
    assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS learning_events"));
    assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS learning_code_proposals"));
    assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS outcome_contracts"));
    assert!(SCHEMA.contains("conversation_messages_fts"));
    assert!(SCHEMA.contains("surface TEXT NOT NULL DEFAULT 'agent_cockpit'"));
    assert!(SCHEMA.contains("idx_conversations_surface_activity"));
}

#[test]
fn upgrades_and_repairs_cover_conversation_surfaces() {
    assert!(UPGRADES.iter().any(|upgrade| {
        upgrade.version == 30
            && upgrade
                .sql
                .contains("ADD COLUMN surface TEXT NOT NULL DEFAULT 'agent_cockpit'")
    }));
    assert!(
        DATA_REPAIRS
            .iter()
            .any(|statement| statement.contains("SET surface = 'agent_cockpit'"))
    );
}

#[test]
fn repairs_rebuild_transcript_fts() {
    assert!(
        DATA_REPAIRS
            .iter()
            .any(|statement| statement.contains("conversation_messages_fts")
                && statement.contains("rebuild"))
    );
}
