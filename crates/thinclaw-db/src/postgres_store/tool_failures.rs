#![cfg_attr(not(feature = "postgres"), allow(unused_imports))]

use super::*;

// ==================== Tool Failures ====================

#[cfg(feature = "postgres")]
use thinclaw_types::BrokenTool;

#[cfg(feature = "postgres")]
impl Store {
    /// Record a tool failure (upsert: increment count if exists).
    pub async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            r#"
            INSERT INTO tool_failures (tool_name, error_message, error_count, last_failure)
            VALUES ($1, $2, 1, NOW())
            ON CONFLICT (tool_name) DO UPDATE SET
                error_message = $2,
                error_count = tool_failures.error_count + 1,
                last_failure = NOW()
            "#,
            &[&tool_name, &error_message],
        )
        .await?;

        Ok(())
    }

    /// Get tools that have failed more than `threshold` times and haven't been repaired.
    pub async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT tool_name, error_message, error_count, first_failure, last_failure,
                       last_build_result, repair_attempts
                FROM tool_failures
                WHERE error_count >= $1 AND repaired_at IS NULL
                ORDER BY error_count DESC
                "#,
                &[&threshold],
            )
            .await?;

        Ok(rows
            .iter()
            .map(|row| BrokenTool {
                name: row.get("tool_name"),
                last_error: row.get("error_message"),
                failure_count: row.get::<_, i32>("error_count") as u32,
                first_failure: row.get("first_failure"),
                last_failure: row.get("last_failure"),
                last_build_result: row.get("last_build_result"),
                repair_attempts: row.get::<_, i32>("repair_attempts") as u32,
            })
            .collect())
    }

    /// Mark a tool as repaired.
    pub async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE tool_failures SET repaired_at = NOW(), error_count = 0 WHERE tool_name = $1",
            &[&tool_name],
        )
        .await?;

        Ok(())
    }

    /// Increment repair attempts for a tool.
    pub async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE tool_failures SET repair_attempts = repair_attempts + 1 WHERE tool_name = $1",
            &[&tool_name],
        )
        .await?;

        Ok(())
    }
}
