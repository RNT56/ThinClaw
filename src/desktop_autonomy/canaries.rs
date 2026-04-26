use super::*;
impl DesktopAutonomyManager {
    pub async fn execute_canary_manifest(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> Result<DesktopCanaryReport, String> {
        let mut checks = Vec::new();
        checks.push(
            match self.bridge_call("health", serde_json::json!({})).await {
                Ok(result) => {
                    self.runtime_passed_check("bridge_health", Some(result.clone()), result)
                }
                Err(err) => {
                    self.runtime_failed_check("bridge_health", err, serde_json::Value::Null)
                }
            },
        );
        checks.push(match self.desktop_permission_status().await {
            Ok(result) => self.runtime_passed_check("permissions", Some(result.clone()), result),
            Err(err) => self.runtime_failed_check("permissions", err, serde_json::Value::Null),
        });
        checks.push(
            match self.apps_action("list", serde_json::json!({})).await {
                Ok(result) => self.runtime_passed_check("apps_list", None, result),
                Err(err) => self.runtime_failed_check("apps_list", err, serde_json::Value::Null),
            },
        );
        checks.push(self.run_calendar_crud_canary(manifest).await);
        checks.push(self.run_numbers_canary(manifest).await);
        checks.push(self.run_pages_canary(manifest).await);
        checks.push(self.run_textedit_canary(manifest).await);

        let report = DesktopCanaryReport {
            build_id: manifest.build_id.clone(),
            generated_at: Utc::now(),
            passed: checks.iter().all(|check| check.passed),
            fixture_paths: manifest.fixture_paths.clone(),
            checks,
        };
        let raw = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("failed to serialize canary report: {e}"))?;
        tokio::fs::write(&manifest.report_path, raw)
            .await
            .map_err(|e| format!("failed to write canary report: {e}"))?;
        Ok(report)
    }

    pub(super) async fn run_calendar_crud_canary(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> AutonomyCheckResult {
        let title = format!("ThinClaw Canary {}", Uuid::new_v4().simple());
        let updated_title = format!("{title} Updated");
        let start = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        let end = (Utc::now() + chrono::Duration::minutes(65)).to_rfc3339();

        let result = async {
            let ensured = self
                .calendar_action(
                    "ensure_calendar",
                    serde_json::json!({ "title": manifest.fixture_paths.calendar_title }),
                )
                .await?;
            let created = self
                .calendar_action(
                    "create",
                    serde_json::json!({
                        "title": title,
                        "calendar": manifest.fixture_paths.calendar_title,
                        "start": start,
                        "end": end,
                        "notes": "ThinClaw desktop canary event",
                    }),
                )
                .await?;
            let event_id = created
                .get("id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "calendar create did not return an id".to_string())?;
            let found = self
                .calendar_action(
                    "find",
                    serde_json::json!({
                        "query": title,
                        "calendar": manifest.fixture_paths.calendar_title,
                    }),
                )
                .await?;
            self.calendar_action(
                "update",
                serde_json::json!({
                    "event_id": event_id,
                    "title": updated_title,
                }),
            )
            .await?;
            self.calendar_action("delete", serde_json::json!({ "event_id": event_id }))
                .await?;
            let after_delete = self
                .calendar_action(
                    "find",
                    serde_json::json!({
                        "query": updated_title,
                        "calendar": manifest.fixture_paths.calendar_title,
                    }),
                )
                .await?;
            Ok::<serde_json::Value, String>(serde_json::json!({
                "calendar": ensured,
                "created": created,
                "found_before_delete": found,
                "found_after_delete": after_delete,
            }))
        }
        .await;

        match result {
            Ok(evidence) => self.runtime_passed_check("calendar_crud", None, evidence),
            Err(err) => self.runtime_failed_check("calendar_crud", err, serde_json::Value::Null),
        }
    }

    pub(super) async fn run_numbers_canary(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> AutonomyCheckResult {
        let Some(numbers_doc) = manifest.fixture_paths.numbers_doc.as_ref() else {
            return self.runtime_failed_check(
                "numbers_open_write_read_export",
                "missing Numbers fixture path".to_string(),
                serde_json::Value::Null,
            );
        };
        let Some(export_dir) = manifest.fixture_paths.export_dir.as_ref() else {
            return self.runtime_failed_check(
                "numbers_open_write_read_export",
                "missing Numbers export dir".to_string(),
                serde_json::Value::Null,
            );
        };
        let export_path = export_dir.join("numbers-canary.csv");
        let marker = format!("canary-{}", Uuid::new_v4().simple());

        let result = async {
            self.numbers_action("open_doc", serde_json::json!({ "path": numbers_doc }))
                .await?;
            self.numbers_action(
                "run_table_action",
                serde_json::json!({
                    "table": "Table 1",
                    "table_action": "clear_range",
                    "range": "A1:B4",
                }),
            )
            .await?;
            self.numbers_action(
                "run_table_action",
                serde_json::json!({
                    "table": "Table 1",
                    "table_action": "add_row_below",
                    "row_index": 1,
                }),
            )
            .await?;
            self.numbers_action(
                "write_range",
                serde_json::json!({
                    "table": "Table 1",
                    "cell": "A1",
                    "value": marker,
                }),
            )
            .await?;
            let read_back = self
                .numbers_action(
                    "read_range",
                    serde_json::json!({
                        "table": "Table 1",
                        "cell": "A1",
                    }),
                )
                .await?;
            let observed = read_back
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !observed.contains(&marker) {
                return Err(format!(
                    "Numbers read-back mismatch: expected marker {marker}"
                ));
            }
            self.numbers_action(
                "set_formula",
                serde_json::json!({
                    "table": "Table 1",
                    "cell": "B1",
                    "value": "=1+1",
                }),
            )
            .await?;
            self.numbers_action("export", serde_json::json!({ "export_path": export_path }))
                .await?;
            if tokio::fs::metadata(&export_path).await.is_err() {
                return Err(format!(
                    "Numbers export was not created at {}",
                    export_path.display()
                ));
            }
            Ok::<serde_json::Value, String>(serde_json::json!({
                "document": numbers_doc,
                "export_path": export_path,
                "read_back": read_back,
            }))
        }
        .await;

        match result {
            Ok(evidence) => {
                self.runtime_passed_check("numbers_open_write_read_export", None, evidence)
            }
            Err(err) => self.runtime_failed_check(
                "numbers_open_write_read_export",
                err,
                serde_json::Value::Null,
            ),
        }
    }

    pub(super) async fn run_pages_canary(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> AutonomyCheckResult {
        let Some(pages_doc) = manifest.fixture_paths.pages_doc.as_ref() else {
            return self.runtime_failed_check(
                "pages_open_insert_find_export",
                "missing Pages fixture path".to_string(),
                serde_json::Value::Null,
            );
        };
        let Some(export_dir) = manifest.fixture_paths.export_dir.as_ref() else {
            return self.runtime_failed_check(
                "pages_open_insert_find_export",
                "missing Pages export dir".to_string(),
                serde_json::Value::Null,
            );
        };
        let export_path = export_dir.join("pages-canary.pdf");
        let marker = format!("ThinClaw Pages {}", Uuid::new_v4().simple());

        let result = async {
            self.pages_action("open_doc", serde_json::json!({ "path": pages_doc }))
                .await?;
            self.pages_action("insert_text", serde_json::json!({ "text": marker }))
                .await?;
            let found = self
                .pages_action("find", serde_json::json!({ "search": marker }))
                .await?;
            if found.get("found").and_then(|value| value.as_bool()) != Some(true) {
                return Err("Pages did not report the inserted marker".to_string());
            }
            self.pages_action("export", serde_json::json!({ "export_path": export_path }))
                .await?;
            if tokio::fs::metadata(&export_path).await.is_err() {
                return Err(format!(
                    "Pages export was not created at {}",
                    export_path.display()
                ));
            }
            Ok::<serde_json::Value, String>(serde_json::json!({
                "document": pages_doc,
                "export_path": export_path,
                "find_result": found,
            }))
        }
        .await;

        match result {
            Ok(evidence) => {
                self.runtime_passed_check("pages_open_insert_find_export", None, evidence)
            }
            Err(err) => self.runtime_failed_check(
                "pages_open_insert_find_export",
                err,
                serde_json::Value::Null,
            ),
        }
    }

    pub(super) async fn run_textedit_canary(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> AutonomyCheckResult {
        let (app_id, app_label) = self.generic_ui_target();
        let textedit_target = manifest
            .fixture_paths
            .textedit_doc
            .clone()
            .unwrap_or_else(|| manifest.shadow_home.join("canary.txt"));
        let marker = format!("{app_label} Canary {}", Uuid::new_v4().simple());
        let result = async {
            self.apps_action("open", serde_json::json!({ "path": textedit_target }))
                .await?;
            self.apps_action("focus", serde_json::json!({ "bundle_id": app_id }))
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            self.ui_action(
                "type_text",
                serde_json::json!({
                    "bundle_id": app_id,
                    "text": marker,
                }),
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let matches = self
                .screen_action("find_text", serde_json::json!({ "query": marker }))
                .await?;
            let found_any = matches
                .get("matches")
                .and_then(|value| value.as_array())
                .is_some_and(|items| !items.is_empty());
            if !found_any {
                return Err(format!(
                    "{app_label} fallback OCR could not find the typed marker"
                ));
            }
            Ok::<serde_json::Value, String>(matches)
        }
        .await;

        match result {
            Ok(evidence) => {
                self.runtime_passed_check("generic_ui_textedit_fallback", None, evidence)
            }
            Err(err) => self.runtime_failed_check(
                "generic_ui_textedit_fallback",
                err,
                serde_json::Value::Null,
            ),
        }
    }
}
