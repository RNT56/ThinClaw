use super::*;

#[derive(Debug, Clone)]
pub(super) struct ResearchSubagentOutput<T> {
    pub(super) value: T,
    pub(super) run_artifact: AgentRunArtifact,
}

#[derive(Debug, Clone)]
pub(super) struct ResearchSubagentError {
    pub(super) message: String,
    pub(super) run_artifact: AgentRunArtifact,
}

impl std::fmt::Display for ResearchSubagentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

#[derive(Debug)]
pub(super) enum ResearchSubagentInvocationError {
    Api(ApiError),
    Run(Box<ResearchSubagentError>),
}

impl From<ApiError> for ResearchSubagentInvocationError {
    fn from(value: ApiError) -> Self {
        Self::Api(value)
    }
}

impl From<ResearchSubagentError> for ResearchSubagentInvocationError {
    fn from(value: ResearchSubagentError) -> Self {
        Self::Run(Box::new(value))
    }
}

#[derive(Debug)]
pub(super) struct CandidateGenerationError {
    pub(super) message: String,
    pub(super) run_artifacts: Vec<AgentRunArtifact>,
}

impl CandidateGenerationError {
    pub(super) fn new(message: impl Into<String>, run_artifacts: Vec<AgentRunArtifact>) -> Self {
        Self {
            message: message.into(),
            run_artifacts,
        }
    }
}

pub fn register_experiment_subagent_executor(executor: Arc<SubagentExecutor>) {
    let _ = RESEARCH_SUBAGENT_EXECUTOR.set(executor);
}

pub fn register_experiment_secrets_store(store: Arc<dyn SecretsStore + Send + Sync>) {
    let _ = RESEARCH_SECRETS_STORE.set(store);
}

pub(super) fn research_subagent_executor() -> Option<Arc<SubagentExecutor>> {
    RESEARCH_SUBAGENT_EXECUTOR.get().cloned()
}

pub(super) fn research_secrets_store() -> Option<Arc<dyn SecretsStore + Send + Sync>> {
    RESEARCH_SECRETS_STORE.get().cloned()
}

pub(super) async fn research_provider_api_key(
    user_id: &str,
    runner: &ExperimentRunnerProfile,
) -> Option<String> {
    if !runner.backend.is_gpu_cloud() {
        return None;
    }
    let secrets = research_secrets_store()?;
    let mut names = Vec::new();
    if let Some(default_name) = adapters::gpu_cloud_secret_name(runner.backend) {
        names.push(default_name.to_string());
    }
    for name in &runner.secret_references {
        if !names.iter().any(|entry| entry == name) {
            names.push(name.clone());
        }
    }
    for name in names {
        match secrets
            .get_for_injection(
                user_id,
                &name,
                crate::secrets::SecretAccessContext::new("experiments.api", "gpu_cloud_credential"),
            )
            .await
        {
            Ok(secret) => return Some(secret.expose().to_string()),
            Err(err) => {
                tracing::debug!(
                    provider = runner.backend.slug(),
                    secret_name = %name,
                    error = %err,
                    "Research provider secret lookup failed"
                );
            }
        }
    }
    None
}
