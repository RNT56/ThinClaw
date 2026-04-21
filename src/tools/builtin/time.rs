//! Time utility tool.

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Utc};
use chrono_tz::Tz;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolMetadata, ToolOutput, ToolRouteIntent, require_str};

/// Tool for getting current time and date operations.
pub struct TimeTool;

struct ResolvedTimezone {
    tz: Tz,
    label: String,
}

fn resolve_requested_timezone(
    params: &serde_json::Value,
    ctx: &JobContext,
) -> Result<ResolvedTimezone, ToolError> {
    match params.get("timezone").and_then(|value| value.as_str()) {
        Some(raw)
            if raw.eq_ignore_ascii_case("local")
                || raw.eq_ignore_ascii_case("user")
                || raw.eq_ignore_ascii_case("default") =>
        {
            let tz = crate::timezone::resolve_effective_timezone(Some(&ctx.user_id), None);
            Ok(ResolvedTimezone {
                tz,
                label: tz.to_string(),
            })
        }
        Some(raw) => {
            let tz = crate::timezone::parse_timezone(raw).ok_or_else(|| {
                ToolError::InvalidParameters(format!(
                    "invalid timezone '{}'; use an IANA timezone like 'Europe/Berlin' or a fixed offset like 'GMT+1'",
                    raw
                ))
            })?;
            let label =
                crate::timezone::normalize_timezone_label(raw).unwrap_or_else(|| tz.to_string());
            Ok(ResolvedTimezone { tz, label })
        }
        None => {
            let tz = crate::timezone::resolve_effective_timezone(Some(&ctx.user_id), None);
            Ok(ResolvedTimezone {
                tz,
                label: tz.to_string(),
            })
        }
    }
}

fn parse_rfc3339_timestamp(
    value: &str,
    field_name: &str,
) -> Result<DateTime<FixedOffset>, ToolError> {
    DateTime::parse_from_rfc3339(value)
        .map_err(|err| ToolError::InvalidParameters(format!("invalid {}: {}", field_name, err)))
}

#[async_trait]
impl Tool for TimeTool {
    fn name(&self) -> &str {
        "time"
    }

    fn description(&self) -> &str {
        "Authoritative time source. Use this whenever you need the current time, \
         date, weekday, or timezone-aware 'now', or when converting and comparing \
         timestamps. Do not infer the current time from the user's timezone alone."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["now", "parse", "format", "diff"],
                    "description": "The time operation to perform"
                },
                "timestamp": {
                    "type": "string",
                    "description": "ISO 8601 timestamp (for parse/format/diff operations)"
                },
                "format": {
                    "type": "string",
                    "description": "Output format string (for format operation)"
                },
                "timestamp2": {
                    "type": "string",
                    "description": "Second timestamp (for diff operation)"
                },
                "timezone": {
                    "type": "string",
                    "description": "Optional timezone for now/parse/format operations. Accepts IANA names like 'Europe/Berlin', fixed offsets like 'GMT+1', or 'local' to use the current effective timezone."
                }
            },
            "required": ["operation"]
        })
    }

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata::live_authoritative(ToolRouteIntent::CurrentTime)
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let operation = require_str(&params, "operation")?;

        let result = match operation {
            "now" => {
                let resolved = resolve_requested_timezone(&params, ctx)?;
                let now = Utc::now();
                let local = now.with_timezone(&resolved.tz);
                serde_json::json!({
                    "iso": now.to_rfc3339(),
                    "unix": now.timestamp(),
                    "unix_millis": now.timestamp_millis(),
                    "timezone": resolved.label,
                    "local_iso": local.to_rfc3339(),
                    "local_date": local.format("%Y-%m-%d").to_string(),
                    "local_time": local.format("%H:%M:%S").to_string()
                })
            }
            "parse" => {
                let timestamp = require_str(&params, "timestamp")?;
                let resolved = resolve_requested_timezone(&params, ctx)?;
                let dt = parse_rfc3339_timestamp(timestamp, "timestamp")?;
                let utc = dt.with_timezone(&Utc);
                let local = dt.with_timezone(&resolved.tz);

                serde_json::json!({
                    "iso": utc.to_rfc3339(),
                    "unix": utc.timestamp(),
                    "unix_millis": utc.timestamp_millis(),
                    "timezone": resolved.label,
                    "local_iso": local.to_rfc3339(),
                    "offset_seconds": dt.offset().local_minus_utc()
                })
            }
            "format" => {
                let timestamp = require_str(&params, "timestamp")?;
                let format = require_str(&params, "format")?;
                let resolved = resolve_requested_timezone(&params, ctx)?;
                let dt = parse_rfc3339_timestamp(timestamp, "timestamp")?;
                let local = dt.with_timezone(&resolved.tz);

                serde_json::json!({
                    "formatted": local.format(format).to_string(),
                    "timezone": resolved.label,
                    "local_iso": local.to_rfc3339(),
                    "iso": dt.with_timezone(&Utc).to_rfc3339()
                })
            }
            "diff" => {
                let ts1 = require_str(&params, "timestamp")?;

                let ts2 = require_str(&params, "timestamp2")?;

                let dt1 = parse_rfc3339_timestamp(ts1, "timestamp")?.with_timezone(&Utc);
                let dt2 = parse_rfc3339_timestamp(ts2, "timestamp2")?.with_timezone(&Utc);

                let diff = dt2.signed_duration_since(dt1);

                serde_json::json!({
                    "seconds": diff.num_seconds(),
                    "minutes": diff.num_minutes(),
                    "hours": diff.num_hours(),
                    "days": diff.num_days()
                })
            }
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown operation: {}",
                    operation
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool, no external data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_requested_timezone_accepts_gmt_offset_aliases() {
        let ctx = JobContext::with_user("time-test", "chat", "time test");
        let resolved = resolve_requested_timezone(&serde_json::json!({"timezone": "GMT+1"}), &ctx)
            .expect("GMT+1 should resolve");
        assert_eq!(
            resolved.tz,
            "Etc/GMT-1".parse::<Tz>().expect("valid test timezone")
        );
        assert_eq!(resolved.label, "GMT+1");
    }

    #[test]
    fn resolve_requested_timezone_accepts_local_alias() {
        let ctx = JobContext::with_user("time-test", "chat", "time test");
        let resolved = resolve_requested_timezone(&serde_json::json!({"timezone": "local"}), &ctx)
            .expect("local should resolve");
        assert!(!resolved.label.is_empty());
    }
}
