use std::sync::Arc;

use crate::config::RepoProjectsConfig;
use crate::db::Database;

pub(super) async fn resolve_repo_projects_config(store: &Arc<dyn Database>) -> RepoProjectsConfig {
    let mut default_config = RepoProjectsConfig::default();

    for user_id in ["default", "local_user"] {
        match store.get_all_settings(user_id).await {
            Ok(map) => {
                let settings = crate::settings::Settings::from_db_map(&map);
                match RepoProjectsConfig::resolve(&settings) {
                    Ok(config) if config.enabled => return config,
                    Ok(config) if user_id == "default" => {
                        default_config = config;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            user_id,
                            error = %error,
                            "failed to resolve repo projects config"
                        );
                    }
                }
            }
            Err(error) => {
                tracing::debug!(
                    user_id,
                    error = %error,
                    "failed to load settings while resolving repo projects config"
                );
            }
        }
    }

    default_config
}
