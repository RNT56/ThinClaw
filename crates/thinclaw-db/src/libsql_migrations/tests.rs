use super::{DATA_REPAIRS, SCHEMA};

#[test]
fn schema_includes_learning_tables() {
    assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS learning_events"));
    assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS learning_code_proposals"));
    assert!(SCHEMA.contains("CREATE TABLE IF NOT EXISTS outcome_contracts"));
    assert!(SCHEMA.contains("conversation_messages_fts"));
}

#[test]
fn repairs_rebuild_transcript_fts() {
    assert!(
        DATA_REPAIRS
            .iter()
            .any(|stmt| stmt.contains("conversation_messages_fts") && stmt.contains("rebuild"))
    );
}
