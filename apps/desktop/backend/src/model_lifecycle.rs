//! Backend-owned serialization and target matching for local model lifecycles.
//!
//! Every model-backed process start/stop, targeted deactivation, and managed
//! model deletion takes this lock. Active and install paths passed to the
//! matching helper are canonical filesystem paths, so component-aware
//! containment cannot confuse sibling names such as `model` and `model-old`.

use std::path::Path;
use std::sync::LazyLock;

pub(crate) static MODEL_LIFECYCLE_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ModelLifecycleRoles {
    pub chat: bool,
    pub embedding: bool,
    pub summarizer: bool,
    pub stt: bool,
    pub image: bool,
    pub tts: bool,
    pub engine: bool,
}

impl ModelLifecycleRoles {
    pub(crate) fn from_deactivation_command(
        chat: bool,
        embedding: bool,
        summarizer: bool,
        stt: bool,
        image: bool,
    ) -> Self {
        Self {
            chat,
            embedding,
            summarizer,
            stt,
            image,
            tts: false,
            // Directory chat runtimes are an implementation detail. The
            // backend checks their actual target when chat is requested.
            engine: chat,
        }
    }

    pub(crate) const fn all() -> Self {
        Self {
            chat: true,
            embedding: true,
            summarizer: true,
            stt: true,
            image: true,
            tts: true,
            engine: true,
        }
    }
}

pub(crate) fn model_path_uses_install(active_path: &Path, install_root: &Path) -> bool {
    active_path == install_root || active_path.starts_with(install_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_target_matching_is_component_aware_and_exact() {
        let install = Path::new("/managed/models/LLM/model-a");
        assert!(model_path_uses_install(install, install));
        assert!(model_path_uses_install(
            Path::new("/managed/models/LLM/model-a/weights/model.gguf"),
            install,
        ));
        assert!(!model_path_uses_install(
            Path::new("/managed/models/LLM/model-ab/model.gguf"),
            install,
        ));
        assert!(!model_path_uses_install(
            Path::new("/managed/models/LLM/model-b/model.gguf"),
            install,
        ));
        assert!(!model_path_uses_install(
            Path::new("/managed/models/LLM"),
            install,
        ));
    }

    #[test]
    fn deactivation_command_role_contract_maps_every_boolean() {
        let roles = ModelLifecycleRoles::from_deactivation_command(true, false, true, false, true);
        assert_eq!(
            roles,
            ModelLifecycleRoles {
                chat: true,
                embedding: false,
                summarizer: true,
                stt: false,
                image: true,
                tts: false,
                engine: true,
            }
        );
        assert_eq!(
            ModelLifecycleRoles::all(),
            ModelLifecycleRoles {
                chat: true,
                embedding: true,
                summarizer: true,
                stt: true,
                image: true,
                tts: true,
                engine: true,
            }
        );
    }
}
