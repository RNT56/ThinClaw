//! Local diffusion backend — wraps existing sd.cpp / mflux sidecar.

use crate::inference::diffusion::{DiffusionBackend, DiffusionRequest, DiffusionResult};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

/// Local diffusion backend using sd.cpp or mflux sidecar.
///
/// Delegates to `image_gen.rs::generate_image()`.  This adapter provides
/// the `DiffusionBackend` trait interface for the InferenceRouter.
pub struct LocalDiffusionBackend {
    /// Whether mflux (MLX) is available.
    pub has_mflux: bool,
    /// Active model (if any).
    pub model: Option<String>,
}

#[async_trait]
impl DiffusionBackend for LocalDiffusionBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "local".to_string(),
            display_name: if self.has_mflux {
                "Local (mflux)".to_string()
            } else {
                "Local (sd.cpp)".to_string()
            },
            is_local: true,
            model_id: self.model.clone(),
            available: true,
        }
    }

    async fn generate(&self, _request: DiffusionRequest) -> InferenceResult<DiffusionResult> {
        // Actual generation is delegated to the existing image_gen.rs flow
        // which is invoked by the direct_imagine_generate command handler.
        Err(InferenceError::other(
            "LocalDiffusionBackend: use direct_imagine_generate command directly",
        ))
    }

    fn max_resolution(&self) -> u32 {
        4096
    }
}
