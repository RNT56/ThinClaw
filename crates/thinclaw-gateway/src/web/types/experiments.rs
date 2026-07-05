//! Experiment query and GPU-cloud provider DTOs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize, Default)]
pub struct ExperimentsQuery {
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExperimentsLimitQuery {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExperimentGpuCloudConnectRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExperimentGpuCloudLaunchTestRequest {
    #[serde(default)]
    pub runner_profile_id: Option<Uuid>,
    #[serde(default)]
    pub gateway_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExperimentGpuCloudTemplateRequest {
    #[serde(default)]
    pub runner_name: Option<String>,
    #[serde(default)]
    pub image_or_runtime: Option<String>,
    #[serde(default)]
    pub region_name: Option<String>,
    #[serde(default)]
    pub instance_type_name: Option<String>,
    #[serde(default = "default_experiment_gpu_cloud_quantity")]
    pub quantity: u32,
    #[serde(default)]
    pub ssh_key_names: Vec<String>,
    #[serde(default)]
    pub file_system_names: Vec<String>,
}

fn default_experiment_gpu_cloud_quantity() -> u32 {
    1
}
