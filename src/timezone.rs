//! Timezone resolution and utilities.
//!
//! Provides timezone detection, resolution with priority chains, markdown
//! field helpers, and time-aware helper functions for the agent system.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

static USER_TIMEZONE_OVERRIDES: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Resolve the effective timezone from a priority chain.
///
/// Priority: `client_tz` > `user_setting` > `config_default` > UTC
pub fn resolve_timezone(
    client_tz: Option<&str>,
    user_setting: Option<&str>,
    config_default: &str,
) -> Tz {
    for candidate in [client_tz, user_setting, Some(config_default)] {
        if let Some(tz) = candidate.and_then(parse_timezone) {
            return tz;
        }
    }
    Tz::UTC
}

fn parse_fixed_offset_alias(s: &str) -> Option<(Tz, String)> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut compact = trimmed.to_ascii_uppercase();
    compact.retain(|ch| !ch.is_ascii_whitespace());

    let (prefix, remainder) = compact
        .strip_prefix("GMT")
        .map(|rest| ("GMT", rest))
        .or_else(|| compact.strip_prefix("UTC").map(|rest| ("UTC", rest)))?;

    if remainder.is_empty() {
        return Some((Tz::UTC, prefix.to_string()));
    }

    let sign = match remainder.chars().next()? {
        '+' => '+',
        '-' => '-',
        _ => return None,
    };
    let digits = &remainder[1..];
    if digits.is_empty() {
        return None;
    }

    let (hours_str, minutes_str) = if let Some((hours, minutes)) = digits.split_once(':') {
        (hours, Some(minutes))
    } else if digits.len() == 4 {
        (&digits[..2], Some(&digits[2..]))
    } else if digits.len() == 3 {
        (&digits[..1], Some(&digits[1..]))
    } else {
        (digits, None)
    };

    if hours_str.is_empty() || !hours_str.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let hours: u32 = hours_str.parse().ok()?;
    let minutes: u32 = match minutes_str {
        Some(minutes) if !minutes.is_empty() && minutes.chars().all(|ch| ch.is_ascii_digit()) => {
            minutes.parse().ok()?
        }
        Some(_) => return None,
        None => 0,
    };

    if hours > 14 || minutes >= 60 || (hours == 14 && minutes != 0) {
        return None;
    }

    if hours == 0 && minutes == 0 {
        return Some((Tz::UTC, prefix.to_string()));
    }

    if minutes != 0 {
        return None;
    }

    // IANA `Etc/GMT` zones use the opposite sign convention from common UTC/GMT offsets.
    let iana_name = match sign {
        '+' => format!("Etc/GMT-{}", hours),
        '-' => format!("Etc/GMT+{}", hours),
        _ => unreachable!("validated sign above"),
    };
    let tz = iana_name.parse::<Tz>().ok()?;
    Some((tz, format!("{prefix}{sign}{hours}")))
}

/// Parse a timezone string (IANA name or common GMT/UTC offset alias) into a `Tz`.
pub fn parse_timezone(s: &str) -> Option<Tz> {
    s.parse::<Tz>()
        .ok()
        .or_else(|| parse_fixed_offset_alias(s).map(|(tz, _)| tz))
}

/// Normalize an accepted timezone input into a durable label.
pub fn normalize_timezone_label(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('_') {
        return None;
    }

    if let Some((_, canonical)) = parse_fixed_offset_alias(trimmed) {
        return Some(canonical);
    }

    parse_timezone(trimmed).map(|tz| tz.to_string())
}

fn normalized_timezone_value(value: &str) -> Option<String> {
    normalize_timezone_label(value)
}

fn markdown_timezone_value(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("- **Timezone:**")
            .or_else(|| trimmed.strip_prefix("- **Timezone**:"))
            .map(str::trim)
    })
}

fn resolve_local_datetime(tz: Tz, naive: NaiveDateTime) -> DateTime<Tz> {
    for minute_offset in 0..=180 {
        let candidate = naive + Duration::minutes(minute_offset);
        match tz.from_local_datetime(&candidate) {
            LocalResult::Single(dt) => return dt,
            LocalResult::Ambiguous(earliest, _) => return earliest,
            LocalResult::None => continue,
        }
    }

    tz.from_utc_datetime(&naive)
}

/// Extract a valid timezone value from a markdown `**Timezone:**` field.
pub fn extract_markdown_timezone(content: &str) -> Option<String> {
    markdown_timezone_value(content).and_then(normalized_timezone_value)
}

/// Validate that a markdown `**Timezone:**` field is either blank or a valid
/// IANA timezone / common GMT/UTC offset alias.
pub fn validate_markdown_timezone_field(content: &str) -> Result<(), String> {
    let Some(value) = markdown_timezone_value(content) else {
        return Ok(());
    };

    if value.is_empty() || value.starts_with('_') {
        return Ok(());
    }

    if parse_timezone(value).is_some() {
        Ok(())
    } else {
        Err(format!(
            "invalid timezone '{}' in USER.md; use an IANA timezone like 'Europe/Berlin' or a fixed offset like 'GMT+1'",
            value
        ))
    }
}

/// Store or clear the live timezone override for a user.
pub fn set_user_timezone_override(user_id: &str, timezone: Option<&str>) -> Result<(), String> {
    let Some(mut overrides) = USER_TIMEZONE_OVERRIDES.write().ok() else {
        return Err("failed to acquire timezone override lock".to_string());
    };

    match timezone {
        Some(value) => {
            let normalized = normalized_timezone_value(value).ok_or_else(|| {
                format!(
                    "invalid timezone '{}'; use an IANA timezone like 'Europe/Berlin' or a fixed offset like 'GMT+1'",
                    value
                )
            })?;
            overrides.insert(user_id.to_string(), normalized);
        }
        None => {
            overrides.remove(user_id);
        }
    }

    Ok(())
}

/// Get the current live timezone override for a user, if one exists.
pub fn user_timezone_override(user_id: Option<&str>) -> Option<String> {
    let user_id = user_id?;
    USER_TIMEZONE_OVERRIDES
        .read()
        .ok()
        .and_then(|overrides| overrides.get(user_id).cloned())
}

/// Resolve the effective timezone for a user.
///
/// Priority: live user override > persisted user setting > system timezone > UTC
pub fn resolve_effective_timezone(user_id: Option<&str>, user_setting: Option<&str>) -> Tz {
    let system_timezone = detect_system_timezone().to_string();
    resolve_timezone(
        user_timezone_override(user_id).as_deref(),
        user_setting,
        &system_timezone,
    )
}

/// Get today's date in the given timezone.
pub fn today_in_tz(tz: Tz) -> NaiveDate {
    Utc::now().with_timezone(&tz).date_naive()
}

/// Get the current time in the given timezone.
pub fn now_in_tz(tz: Tz) -> DateTime<Tz> {
    Utc::now().with_timezone(&tz)
}

/// Get today's date for a user in their effective timezone.
pub fn today_for_user(user_id: Option<&str>, user_setting: Option<&str>) -> NaiveDate {
    today_in_tz(resolve_effective_timezone(user_id, user_setting))
}

/// Get the current time for a user in their effective timezone.
pub fn now_for_user(user_id: Option<&str>, user_setting: Option<&str>) -> DateTime<Tz> {
    now_in_tz(resolve_effective_timezone(user_id, user_setting))
}

/// Get the UTC timestamp corresponding to the start of a local day for a user.
pub fn local_day_start_utc(
    user_id: Option<&str>,
    user_setting: Option<&str>,
    day: NaiveDate,
) -> DateTime<Utc> {
    let tz = resolve_effective_timezone(user_id, user_setting);
    let start = resolve_local_datetime(
        tz,
        day.and_hms_opt(0, 0, 0)
            .expect("midnight is always a valid NaiveDateTime"),
    );
    start.with_timezone(&Utc)
}

/// Detect the system's timezone, falling back to UTC.
pub fn detect_system_timezone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|s| parse_timezone(&s))
        .unwrap_or(Tz::UTC)
}

/// Recompute stored `next_fire_at` values for a user's scheduled routines.
///
/// When `preserve_due` is true, routines that are already due are left untouched
/// so startup rehydration does not skip imminent runs.
pub async fn refresh_user_routine_timezones(
    store: &Arc<dyn crate::db::Database>,
    user_id: &str,
    user_setting: Option<&str>,
    preserve_due: bool,
) -> Result<usize, String> {
    let now = Utc::now();
    let routines = store
        .list_routines(user_id)
        .await
        .map_err(|err| format!("failed to load routines: {err}"))?;
    let mut updated = 0usize;

    for mut routine in routines {
        let uses_timezone = match crate::agent::routine::routine_schedule_uses_timezone(&routine) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(
                    routine = %routine.name,
                    error = %err,
                    "Skipping routine timezone refresh for invalid schedule"
                );
                continue;
            }
        };

        if !uses_timezone && routine.next_fire_at.is_some() {
            continue;
        }

        if preserve_due && routine.enabled && routine.next_fire_at.is_some_and(|ts| ts <= now) {
            continue;
        }

        let next_fire =
            match crate::agent::routine::next_fire_for_routine(&routine, user_setting, now) {
                Ok(next_fire) => next_fire,
                Err(err) => {
                    tracing::warn!(
                        routine = %routine.name,
                        error = %err,
                        "Skipping routine timezone refresh for invalid schedule"
                    );
                    continue;
                }
            };

        if routine.next_fire_at == next_fire {
            continue;
        }

        routine.next_fire_at = next_fire;
        routine.updated_at = now;
        store
            .update_routine(&routine)
            .await
            .map_err(|err| format!("failed to update routine '{}': {err}", routine.name))?;
        updated += 1;
    }

    Ok(updated)
}

/// Persist, activate, and propagate a user's effective timezone.
pub async fn apply_user_timezone_change(
    store: &Arc<dyn crate::db::Database>,
    workspace: Option<&crate::workspace::Workspace>,
    user_id: &str,
    timezone: Option<&str>,
) -> Result<Option<String>, String> {
    let normalized = match timezone {
        Some(value) => Some(normalized_timezone_value(value).ok_or_else(|| {
            format!(
                "invalid timezone '{}'; use an IANA timezone like 'Europe/Berlin' or a fixed offset like 'GMT+1'",
                value
            )
        })?),
        None => None,
    };

    match normalized.as_deref() {
        Some(value) => store
            .set_setting(user_id, "user_timezone", &serde_json::json!(value))
            .await
            .map_err(|err| format!("failed to persist timezone setting: {err}"))?,
        None => {
            store
                .delete_setting(user_id, "user_timezone")
                .await
                .map_err(|err| {
                    format!("failed to clear timezone setting for '{}': {err}", user_id)
                })?;
        }
    }

    set_user_timezone_override(user_id, normalized.as_deref())?;

    if let Some(workspace) = workspace {
        workspace
            .sync_user_timezone(normalized.as_deref())
            .await
            .map_err(|err| format!("failed to sync USER.md timezone: {err}"))?;
    }

    refresh_user_routine_timezones(store, user_id, normalized.as_deref(), false).await?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use chrono::Datelike;

    use super::*;

    #[test]
    fn test_resolve_client_wins() {
        let tz = resolve_timezone(Some("America/New_York"), Some("Europe/London"), "UTC");
        assert_eq!(tz, chrono_tz::America::New_York);
    }

    #[test]
    fn test_resolve_user_setting_fallback() {
        let tz = resolve_timezone(None, Some("Europe/London"), "UTC");
        assert_eq!(tz, chrono_tz::Europe::London);
    }

    #[test]
    fn test_resolve_config_fallback() {
        let tz = resolve_timezone(None, None, "Asia/Tokyo");
        assert_eq!(tz, chrono_tz::Asia::Tokyo);
    }

    #[test]
    fn test_resolve_all_none_utc() {
        let tz = resolve_timezone(None, None, "UTC");
        assert_eq!(tz, Tz::UTC);
    }

    #[test]
    fn test_resolve_invalid_client_skipped() {
        let tz = resolve_timezone(Some("Fake/Zone"), Some("Europe/London"), "UTC");
        assert_eq!(tz, chrono_tz::Europe::London);
    }

    #[test]
    fn test_parse_valid() {
        assert_eq!(
            parse_timezone("America/Chicago"),
            Some(chrono_tz::America::Chicago)
        );
    }

    #[test]
    fn test_parse_gmt_offset_alias() {
        assert_eq!(
            parse_timezone("GMT+1"),
            Some("Etc/GMT-1".parse::<Tz>().expect("valid test timezone"))
        );
        assert_eq!(
            parse_timezone("UTC-5"),
            Some("Etc/GMT+5".parse::<Tz>().expect("valid test timezone"))
        );
        assert_eq!(
            parse_timezone("gmt +01:00"),
            Some("Etc/GMT-1".parse::<Tz>().expect("valid test timezone"))
        );
    }

    #[test]
    fn test_parse_invalid() {
        assert_eq!(parse_timezone("Fake/Zone"), None);
        assert_eq!(parse_timezone("GMT+5:30"), None);
    }

    #[test]
    fn test_extract_markdown_timezone_valid() {
        let tz = extract_markdown_timezone("- **Timezone:** Europe/Berlin\n");
        assert_eq!(tz.as_deref(), Some("Europe/Berlin"));
    }

    #[test]
    fn test_extract_markdown_timezone_accepts_gmt_offset() {
        let tz = extract_markdown_timezone("- **Timezone:** GMT+1\n");
        assert_eq!(tz.as_deref(), Some("GMT+1"));
    }

    #[test]
    fn test_extract_markdown_timezone_ignores_invalid() {
        let tz = extract_markdown_timezone("- **Timezone:** Fake/Zone\n");
        assert!(tz.is_none());
    }

    #[test]
    fn test_validate_markdown_timezone_field_rejects_invalid() {
        let result = validate_markdown_timezone_field("- **Timezone:** Fake/Zone\n");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_markdown_timezone_field_accepts_gmt_offset() {
        let result = validate_markdown_timezone_field("- **Timezone:** GMT+1\n");
        assert!(result.is_ok());
    }

    #[test]
    fn test_live_user_override_wins() {
        set_user_timezone_override("tz-test", Some("Asia/Tokyo")).expect("set override");
        let tz = resolve_effective_timezone(Some("tz-test"), Some("Europe/Berlin"));
        assert_eq!(tz, chrono_tz::Asia::Tokyo);
        set_user_timezone_override("tz-test", None).expect("clear override");
    }

    #[test]
    fn test_detect_system_tz() {
        let tz = detect_system_timezone();
        let _ = now_in_tz(tz); // Should not panic
    }

    #[test]
    fn test_today_in_tz_returns_valid_date() {
        let date = today_in_tz(Tz::UTC);
        assert!(date.year() > 0);
        assert!((1..=12).contains(&date.month()));
        assert!((1..=31).contains(&date.day()));
    }
}
