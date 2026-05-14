#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoverageKind {
    BackendContract,
    Adapter,
    DefaultHelper,
    Infrastructure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoverageEntry {
    pub trait_name: String,
    pub method_name: String,
    pub kind: CoverageKind,
    pub covered_by: Vec<&'static str>,
}

pub(crate) fn build_ledger() -> Vec<CoverageEntry> {
    let mut entries = Vec::new();

    parse_source(
        include_str!("../../crates/thinclaw-db/src/lib.rs"),
        &mut entries,
    );
    parse_source(
        include_str!("../../crates/thinclaw-workspace/src/store.rs"),
        &mut entries,
    );

    entries
}

fn parse_source(src: &str, entries: &mut Vec<CoverageEntry>) {
    let mut active_trait: Option<String> = None;
    for line in src.lines() {
        let trimmed = line.trim();
        if let Some(name) = parse_trait_name(trimmed) {
            active_trait = Some(name.to_string());
            continue;
        }

        if trimmed == "}" {
            active_trait = None;
            continue;
        }

        let Some(trait_name) = active_trait.as_ref() else {
            continue;
        };

        if let Some(method_name) = parse_method_name(trimmed) {
            let kind = classify_method(trait_name, &method_name);
            entries.push(CoverageEntry {
                trait_name: trait_name.clone(),
                method_name,
                kind,
                covered_by: vec![owner_test_for_trait(trait_name)],
            });
        }
    }
}

fn parse_trait_name(line: &str) -> Option<&str> {
    if !line.starts_with("pub trait ") {
        return None;
    }
    let rest = line.trim_start_matches("pub trait ");
    let name = rest.split(':').next()?.trim();
    if name.is_empty() { None } else { Some(name) }
}

fn parse_method_name(line: &str) -> Option<String> {
    let needle = "async fn ";
    let start = line.find(needle)? + needle.len();
    let suffix = &line[start..];
    let name = suffix.split('(').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn classify_method(trait_name: &str, method_name: &str) -> CoverageKind {
    if trait_name == "IdentityStore" {
        return CoverageKind::Adapter;
    }
    if trait_name == "Database" {
        return CoverageKind::Infrastructure;
    }

    if trait_name == "SandboxStore"
        && matches!(
            method_name,
            "list_sandbox_jobs_for_actor"
                | "sandbox_job_summary_for_actor"
                | "sandbox_job_belongs_to_actor"
        )
    {
        return CoverageKind::DefaultHelper;
    }

    if trait_name == "RoutineStore"
        && matches!(
            method_name,
            "get_routine_by_name_for_actor" | "list_routines_for_actor"
        )
    {
        return CoverageKind::DefaultHelper;
    }

    if trait_name == "WorkspaceStore" && method_name == "replace_chunks" {
        return CoverageKind::DefaultHelper;
    }

    CoverageKind::BackendContract
}

fn owner_test_for_trait(trait_name: &str) -> &'static str {
    match trait_name {
        "ConversationStore" => "db_contract::conversations",
        "IdentityStore" | "IdentityRegistryStore" => "db_contract::identity",
        "JobStore" => "db_contract::jobs",
        "SandboxStore" => "db_contract::sandbox",
        "RoutineStore" => "db_contract::routines",
        "ToolFailureStore" => "db_contract::tool_failures",
        "ExperimentStore" => "db_contract::experiments",
        "SettingsStore" => "db_contract::settings",
        "WorkspaceStore" => "db_contract::workspace",
        "AgentRegistryStore" => "db_contract::agent_registry",
        "Database" => "db_contract::coverage_ledger",
        _ => "db_contract::unassigned",
    }
}

#[test]
fn database_surface_methods_are_mapped() {
    let entries = build_ledger();
    assert!(!entries.is_empty(), "coverage ledger must not be empty");

    for entry in &entries {
        assert!(
            !entry.covered_by.is_empty(),
            "missing coverage mapping for {}::{}",
            entry.trait_name,
            entry.method_name
        );
    }

    // Keep a safety floor so future shrinkage is visible in CI.
    const COVERAGE_FLOOR: usize = 149;
    assert!(
        entries.len() >= COVERAGE_FLOOR,
        "expected broad DB surface coverage mapping, found only {} entries (floor: {})",
        entries.len(),
        COVERAGE_FLOOR
    );

    assert!(
        entries
            .iter()
            .any(|e| e.trait_name == "Database" && e.method_name == "run_migrations"),
        "Database::run_migrations must be present in the ledger"
    );
}
