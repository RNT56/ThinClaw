use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn ensure_canary_fixtures(&self) -> Result<DesktopFixturePaths, String> {
        self.ensure_dirs().await?;
        let fixtures_dir = self.fixtures_dir();
        tokio::fs::create_dir_all(&fixtures_dir)
            .await
            .map_err(|e| format!("failed to create canary fixtures dir: {e}"))?;

        let (numbers_ext, pages_ext) = self.fixture_extensions();
        let numbers_doc = fixtures_dir.join(format!("canary.{numbers_ext}"));
        let pages_doc = fixtures_dir.join(format!("canary.{pages_ext}"));
        let textedit_doc = fixtures_dir.join("canary.txt");
        let export_dir = fixtures_dir.join("exports");
        tokio::fs::create_dir_all(&export_dir)
            .await
            .map_err(|e| format!("failed to create canary export dir: {e}"))?;
        if tokio::fs::metadata(&textedit_doc).await.is_err() {
            tokio::fs::write(&textedit_doc, "")
                .await
                .map_err(|e| format!("failed to create TextEdit canary fixture: {e}"))?;
        }

        let calendar_title = "ThinClaw Canary".to_string();
        self.bridge_domain_action(
            "calendar",
            "ensure_calendar",
            serde_json::json!({ "title": calendar_title }),
            false,
        )
        .await?;

        if tokio::fs::metadata(&numbers_doc).await.is_err() {
            self.bridge_domain_action(
                "numbers",
                "create_doc",
                serde_json::json!({ "path": numbers_doc }),
                false,
            )
            .await?;
        }

        if tokio::fs::metadata(&pages_doc).await.is_err() {
            self.bridge_domain_action(
                "pages",
                "create_doc",
                serde_json::json!({ "path": pages_doc }),
                false,
            )
            .await?;
        }

        Ok(DesktopFixturePaths {
            calendar_title: "ThinClaw Canary".to_string(),
            numbers_doc: Some(numbers_doc),
            pages_doc: Some(pages_doc),
            textedit_doc: Some(textedit_doc),
            export_dir: Some(export_dir),
        })
    }

    pub(super) async fn write_canary_manifest(
        &self,
        _user_id: &str,
        proposal_id: Uuid,
        build_id: &str,
        build_dir: &Path,
    ) -> Result<DesktopCanaryManifest, String> {
        let live_fixtures = self.ensure_canary_fixtures().await?;
        let shadow_home = build_dir.join("shadow-home");
        let shadow_fixtures_dir = build_dir.join("canary-fixtures");
        let shadow_export_dir = shadow_fixtures_dir.join("exports");
        tokio::fs::create_dir_all(&shadow_home)
            .await
            .map_err(|e| format!("failed to create shadow home: {e}"))?;
        tokio::fs::create_dir_all(&shadow_export_dir)
            .await
            .map_err(|e| format!("failed to create canary export dir: {e}"))?;

        let (numbers_ext, pages_ext) = self.fixture_extensions();
        let numbers_doc = shadow_fixtures_dir.join(format!("canary.{numbers_ext}"));
        let pages_doc = shadow_fixtures_dir.join(format!("canary.{pages_ext}"));
        let textedit_doc = shadow_fixtures_dir.join("canary.txt");

        tokio::fs::create_dir_all(&shadow_fixtures_dir)
            .await
            .map_err(|e| format!("failed to create build fixture dir: {e}"))?;

        if let Some(source) = live_fixtures.numbers_doc.as_ref() {
            copy_fixture_path(source, &numbers_doc)
                .map_err(|e| format!("failed to copy Numbers canary fixture: {e}"))?;
        }
        if let Some(source) = live_fixtures.pages_doc.as_ref() {
            copy_fixture_path(source, &pages_doc)
                .map_err(|e| format!("failed to copy Pages canary fixture: {e}"))?;
        }
        if let Some(source) = live_fixtures.textedit_doc.as_ref() {
            copy_fixture_path(source, &textedit_doc)
                .map_err(|e| format!("failed to copy TextEdit canary fixture: {e}"))?;
        }

        let manifest = DesktopCanaryManifest {
            build_id: build_id.to_string(),
            proposal_id: proposal_id.to_string(),
            report_path: build_dir.join("canary-report.json"),
            shadow_home,
            session_id: self.default_session_id(),
            fixture_paths: DesktopFixturePaths {
                calendar_title: live_fixtures.calendar_title,
                numbers_doc: Some(numbers_doc),
                pages_doc: Some(pages_doc),
                textedit_doc: Some(textedit_doc),
                export_dir: Some(shadow_export_dir),
            },
        };
        let manifest_path = build_dir.join("canary-manifest.json");
        let raw = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("failed to serialize canary manifest: {e}"))?;
        tokio::fs::write(&manifest_path, raw)
            .await
            .map_err(|e| format!("failed to write canary manifest: {e}"))?;
        Ok(manifest)
    }

    pub(super) async fn run_canaries(
        &self,
        build_dir: &Path,
        manifest: &DesktopCanaryManifest,
    ) -> DesktopCanaryReport {
        match self
            .run_shadow_canary_process(&self.shadow_binary_path(build_dir), manifest)
            .await
        {
            Ok(report) => report,
            Err(err) => DesktopCanaryReport {
                build_id: manifest.build_id.clone(),
                generated_at: Utc::now(),
                passed: false,
                fixture_paths: manifest.fixture_paths.clone(),
                checks: vec![self.runtime_failed_check(
                    "shadow_canary_runner",
                    err,
                    serde_json::json!({
                        "binary": self.shadow_binary_path(build_dir),
                        "manifest": build_dir.join("canary-manifest.json"),
                    }),
                )],
            },
        }
    }

    pub(super) async fn run_shadow_canary_process(
        &self,
        binary_path: &Path,
        manifest: &DesktopCanaryManifest,
    ) -> Result<DesktopCanaryReport, String> {
        let mut command = Command::new(binary_path);
        command.arg("autonomy-shadow-canary");
        command.arg("--manifest");
        command.arg(manifest.report_path.with_file_name("canary-manifest.json"));
        command.env("THINCLAW_HOME", &manifest.shadow_home);
        command.env("HOME", &manifest.shadow_home);
        command.env("USERPROFILE", &manifest.shadow_home);
        command.env("DESKTOP_AUTONOMY_ENABLED", "true");
        command.env("DESKTOP_AUTONOMY_PROFILE", self.config.profile.as_str());
        command.env(
            "DESKTOP_AUTONOMY_DEPLOYMENT_MODE",
            self.config.deployment_mode.as_str(),
        );
        if let Some(username) = self.config.target_username.as_deref() {
            command.env("DESKTOP_AUTONOMY_TARGET_USERNAME", username);
        }
        command.env(
            "DESKTOP_AUTONOMY_MAX_CONCURRENT_JOBS",
            self.config.desktop_max_concurrent_jobs.to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_ACTION_TIMEOUT_SECS",
            self.config.desktop_action_timeout_secs.to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_CAPTURE_EVIDENCE",
            self.config.capture_evidence.to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH",
            self.config.emergency_stop_path.as_os_str(),
        );
        if let Some(db) = self.database_config.as_ref() {
            self.apply_shadow_database_env(&mut command, db);
            if matches!(db.backend, crate::config::DatabaseBackend::LibSql) {
                command.env("LIBSQL_PATH", manifest.shadow_home.join("thinclaw.db"));
            }
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = command
            .output()
            .await
            .map_err(|e| format!("failed to spawn shadow canary runner: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                format!("shadow canary runner exited with {}", output.status)
            } else {
                stderr
            });
        }

        serde_json::from_slice::<DesktopCanaryReport>(&output.stdout)
            .map_err(|e| format!("failed to decode shadow canary report: {e}"))
    }
}
