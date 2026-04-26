use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn seed_default_skills(&self) -> Result<Vec<PathBuf>, String> {
        let skills_dir = crate::platform::state_paths()
            .skills_dir
            .join("desktop_autonomy");
        tokio::fs::create_dir_all(&skills_dir)
            .await
            .map_err(|e| format!("failed to create autonomy skills dir: {e}"))?;

        let templates = [
            (
                "desktop_recover_app.md",
                "# desktop_recover_app\n\nRecover a stuck desktop flow by refocusing the target app, dismissing blocking dialogs, reopening the document if needed, restarting the desktop sidecar as a last resort, and only then surfacing attention.\n",
            ),
            (
                "calendar_reconcile.md",
                "# calendar_reconcile\n\nUse `desktop_calendar_native` first. Prefer idempotent find-or-update behavior, verify the final event state, and capture before/after evidence when anything changed.\n",
            ),
            (
                "numbers_update_sheet.md",
                "# numbers_update_sheet\n\nUse `desktop_numbers_native` before generic UI actions. Verify cell reads after every write and prefer table/range operations over coordinate clicks.\n",
            ),
            (
                "pages_prepare_report.md",
                "# pages_prepare_report\n\nUse `desktop_pages_native` before fallback UI automation. Keep edits deterministic, verify exports exist, and preserve document formatting where possible.\n",
            ),
            (
                "daily_desktop_heartbeat.md",
                "# daily_desktop_heartbeat\n\nInspect the desktop autonomy status, confirm bootstrap health, check the emergency stop state, and queue or resume the next desktop routines only when the autonomy profile is healthy.\n",
            ),
        ];

        let mut written = Vec::new();
        for (name, content) in templates {
            let path = skills_dir.join(name);
            tokio::fs::write(&path, content).await.map_err(|e| {
                format!(
                    "failed to seed desktop skill template {}: {e}",
                    path.display()
                )
            })?;
            written.push(path);
        }
        Ok(written)
    }

    pub(super) async fn seed_default_routines(&self) -> Result<Vec<String>, String> {
        let Some(store) = self.store.as_ref() else {
            return Ok(Vec::new());
        };

        let user_id = "default";
        let actor_id = "default";
        let mut created = Vec::new();
        let weekday_nine = canonicalize_schedule_expr("0 0 9 * * MON-FRI *")
            .map_err(|e| format!("failed to build default heartbeat schedule: {e}"))?;
        let heartbeat_next_fire =
            next_schedule_fire_for_user(&weekday_nine, user_id, None).unwrap_or(None);

        let routines = vec![
            Routine {
                id: Uuid::new_v4(),
                name: "desktop_recover_app".to_string(),
                description: "Recover a stuck desktop app flow by refocusing the app, dismissing blocking UI, reopening the working document, and only then escalating for attention.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Recover desktop app".to_string(),
                    description: "Use desktop_apps, desktop_ui, and desktop_screen to recover a stuck desktop application flow. Refocus the target app, dismiss obvious modal blockers, reopen the working document if needed, verify the UI is responsive again, and surface attention only if recovery fails.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_apps".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "calendar_reconcile".to_string(),
                description: "Open or inspect Calendar data, reconcile the requested changes, verify the event state, and preserve evidence for any modifications.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Reconcile Calendar".to_string(),
                    description: "Use desktop_calendar_native first, then desktop_ui and desktop_screen only if needed. Find the target events, apply the requested create/update/delete actions, verify the final event state, and record before/after evidence for any modifications.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_calendar_native".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "desktop_apps".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "numbers_update_sheet".to_string(),
                description: "Open a Numbers document, apply deterministic cell or formula changes, and verify the resulting sheet state.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Update Numbers sheet".to_string(),
                    description: "Use desktop_numbers_native before any fallback desktop_ui actions. Open the requested document, read the target cells, apply writes or formulas, verify the resulting values, and export or save only when requested.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_numbers_native".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "desktop_apps".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "pages_prepare_report".to_string(),
                description: "Open a Pages document, apply the requested textual edits, and verify the resulting export or document state.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Prepare Pages report".to_string(),
                    description: "Use desktop_pages_native first, then desktop_ui and desktop_screen only if needed. Open the requested document, make deterministic text edits, verify the final content, and export the result when requested.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_pages_native".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "desktop_apps".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "daily_desktop_heartbeat".to_string(),
                description: "Weekday desktop heartbeat that inspects autonomy health, checks the emergency stop state, and queues the next desktop routines only when the profile is healthy.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Cron {
                    schedule: weekday_nine.clone(),
                },
                action: RoutineAction::Heartbeat {
                    light_context: true,
                    prompt: Some("Inspect the reckless desktop autonomy status, confirm the bootstrap and permission state are healthy, check the emergency-stop file, and summarize whether desktop routines should continue running today. Queue or recommend any needed follow-up desktop work only when the autonomy profile is healthy.".to_string()),
                    include_reasoning: false,
                    active_start_hour: Some(8),
                    active_end_hour: Some(20),
                    target: "none".to_string(),
                    max_iterations: 8,
                    interval_secs: None,
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: heartbeat_next_fire,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        for routine in routines {
            let exists = store
                .get_routine_by_name_for_actor(user_id, actor_id, &routine.name)
                .await
                .map_err(|e| format!("failed to check routine {}: {e}", routine.name))?
                .is_some();
            if exists {
                continue;
            }
            store
                .create_routine(&routine)
                .await
                .map_err(|e| format!("failed to seed routine {}: {e}", routine.name))?;
            created.push(routine.name);
        }

        Ok(created)
    }

    pub(super) fn fixtures_dir(&self) -> PathBuf {
        self.state_root.join("fixtures")
    }
}
