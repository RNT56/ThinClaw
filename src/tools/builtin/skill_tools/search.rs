//! Skill tool: search.

use super::*;

pub struct SkillSearchTool {
    registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
    catalog: Arc<SkillCatalog>,
    remote_hub: Option<SharedRemoteSkillHub>,
}

impl SkillSearchTool {
    pub fn new(
        registry: Arc<tokio::sync::RwLock<SkillRegistry>>,
        catalog: Arc<SkillCatalog>,
        remote_hub: Option<SharedRemoteSkillHub>,
    ) -> Self {
        Self {
            registry,
            catalog,
            remote_hub,
        }
    }
}

#[async_trait]
impl Tool for SkillSearchTool {
    fn name(&self) -> &str {
        "skill_search"
    }

    fn description(&self) -> &str {
        "Search for skills in the ClawHub catalog and among locally loaded skills."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        skill_policy::skill_search_parameters_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        ensure_skill_admin_available(ctx, self.name())?;
        let start = std::time::Instant::now();
        let parsed = skill_policy::parse_skill_search_params(&params)?;
        let query = parsed.query.as_str();
        let source_filter = parsed.source;

        // Search the ClawHub catalog (async, best-effort)
        let catalog_outcome = self.catalog.search(query).await;
        let catalog_error = catalog_outcome.error.clone();

        // Enrich top results with detail data (stars, downloads, owner)
        let mut catalog_entries = catalog_outcome.results;
        self.catalog
            .enrich_search_results(&mut catalog_entries, 5)
            .await;

        // IC-026: Single lock acquisition for both installed names and local search
        let (installed_names, local_matches): (Vec<String>, Vec<serde_json::Value>) = {
            let guard = self.registry.read().await;

            let names = guard
                .skills()
                .iter()
                .map(|s| s.manifest.name.clone())
                .collect();

            let query_lower = query.to_lowercase();
            let matches = guard
                .skills()
                .iter()
                .filter(|s| {
                    s.manifest.name.to_lowercase().contains(&query_lower)
                        || s.manifest.description.to_lowercase().contains(&query_lower)
                        || s.manifest
                            .activation
                            .keywords
                            .iter()
                            .any(|k| k.to_lowercase().contains(&query_lower))
                })
                .map(|s| {
                    skill_policy::skill_search_local_entry(
                        &s.manifest.name,
                        &s.manifest.description,
                        &s.trust.to_string(),
                        &s.source_tier.to_string(),
                    )
                })
                .collect();

            (names, matches)
        };

        // Mark catalog entries that are already installed
        let catalog_json: Vec<serde_json::Value> = catalog_entries
            .iter()
            .map(|entry| {
                let is_installed = installed_names.iter().any(|n| {
                    // Match by slug suffix or exact name
                    entry.slug.ends_with(n.as_str()) || entry.name == *n
                });
                skill_policy::skill_search_catalog_entry(
                    &entry.slug,
                    &entry.name,
                    &entry.description,
                    &entry.version,
                    entry.score,
                    is_installed,
                    entry.stars,
                    entry.downloads,
                    entry.owner.as_deref(),
                )
            })
            .collect();

        let remote_json = if let Some(ref hub) = self.remote_hub {
            hub.search(query)
                .await
                .into_iter()
                .map(|entry| {
                    let trust_level = format!("{:?}", entry.trust_level).to_lowercase();
                    skill_policy::skill_search_remote_entry(
                        &entry.slug,
                        &entry.name,
                        &entry.description,
                        &entry.version,
                        &entry.source_adapter,
                        &entry.source_label,
                        &entry.source_ref,
                        entry.manifest_url.as_deref(),
                        entry.manifest_digest.as_deref(),
                        entry.repo.as_deref(),
                        entry.path.as_deref(),
                        entry.branch.as_deref(),
                        &trust_level,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let output = skill_policy::skill_search_output(
            source_filter.as_str(),
            catalog_json,
            remote_json,
            local_matches,
            &self.catalog.registry_url(),
            catalog_error,
        );

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── skill_check ──────────────────────────────────────────────────────────
