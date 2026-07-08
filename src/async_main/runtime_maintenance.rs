use std::sync::Arc;

use thinclaw::app::PeriodicPersistencePlan;
use thinclaw::config::Config;

const RUNTIME_MAINTENANCE_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Default)]
pub(super) struct RuntimeHotReloadWatchers {
    pub(super) tool: Option<thinclaw::tools::wasm::ToolWatcher>,
    pub(super) skill: Option<thinclaw::skills::SkillWatcher>,
    pub(super) channel: Option<thinclaw::channels::wasm::channel_watcher::ChannelWatcher>,
}

impl RuntimeHotReloadWatchers {
    pub(super) async fn stop(&mut self) {
        if let Some(watcher) = self.channel.take() {
            watcher.stop().await;
        }
        if let Some(watcher) = self.tool.take() {
            watcher.stop().await;
        }
        if let Some(watcher) = self.skill.take() {
            watcher.stop().await;
        }
    }
}

pub(super) struct RuntimeMaintenanceTask {
    pub(super) name: &'static str,
    pub(super) shutdown_tx: tokio::sync::oneshot::Sender<()>,
    pub(super) handle: tokio::task::JoinHandle<()>,
}

impl RuntimeMaintenanceTask {
    async fn shutdown(self) {
        let Self {
            name,
            shutdown_tx,
            mut handle,
        } = self;
        let _ = shutdown_tx.send(());
        tokio::select! {
            result = &mut handle => {
                if let Err(error) = result {
                    tracing::warn!(task = name, error = %error, "Runtime maintenance task exited with error");
                } else {
                    tracing::debug!(task = name, "Runtime maintenance task drained on shutdown");
                }
            }
            _ = tokio::time::sleep(RUNTIME_MAINTENANCE_SHUTDOWN_TIMEOUT) => {
                handle.abort();
                let _ = handle.await;
                tracing::warn!(task = name, "Runtime maintenance task did not drain before timeout; aborted");
            }
        }
    }
}

pub(super) async fn shutdown_runtime_maintenance(tasks: Vec<RuntimeMaintenanceTask>) {
    for task in tasks {
        task.shutdown().await;
    }
}

/// Spawn the periodic cost-tracker persistence loop (flushes to the DB every
/// 60s so cost data survives restarts). No-op without a DB.
pub(super) fn start_cost_persistence(
    db: &Option<Arc<dyn thinclaw::db::Database>>,
    cost_tracker: &Arc<tokio::sync::Mutex<thinclaw::llm::cost_tracker::CostTracker>>,
) -> Option<RuntimeMaintenanceTask> {
    if let Some(db) = db {
        let persistence_plan = PeriodicPersistencePlan::cost_entries();
        let persist_db = Arc::clone(db);
        let persist_tracker = Arc::clone(cost_tracker);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(persistence_plan.interval);
            interval.tick().await; // skip the initial immediate tick
            let mut last_count: usize = 0;
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        tracing::debug!("[cost] Cost persistence loop stopped");
                        break;
                    }
                    _ = interval.tick() => {}
                }
                let (snapshot, count) = {
                    let guard = persist_tracker.lock().await;
                    (guard.to_json(), guard.entry_count())
                };
                // Only write when new entries have been recorded.
                if count != last_count {
                    match persist_db
                        .set_setting("default", persistence_plan.setting_key, &snapshot)
                        .await
                    {
                        Ok(()) => {
                            tracing::debug!("[cost] Persisted {} cost entries to DB", count);
                            last_count = count;
                        }
                        Err(e) => {
                            tracing::warn!("[cost] Failed to persist cost entries: {}", e);
                        }
                    }
                }
            }
        });
        tracing::info!("Cost persistence background task started (60s interval)");
        Some(RuntimeMaintenanceTask {
            name: "cost_persistence",
            shutdown_tx,
            handle,
        })
    } else {
        None
    }
}

/// Spawn the background pricing sync (loads the DB cache for instant
/// availability, then refreshes from OpenRouter every 24h).
pub(super) fn start_pricing_sync(
    db: &Option<Arc<dyn thinclaw::db::Database>>,
) -> RuntimeMaintenanceTask {
    let pricing_db = db.as_ref().map(Arc::clone);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let handle =
        thinclaw::llm::pricing_sync::spawn_pricing_sync_with_shutdown(pricing_db, shutdown_rx);
    tracing::info!("Pricing sync background task started (24h interval)");
    RuntimeMaintenanceTask {
        name: "pricing_sync",
        shutdown_tx,
        handle,
    }
}

/// Spawn the experiment controller reconciler + artifact reaper when experiments
/// are enabled; otherwise log that they are off.
pub(super) fn start_experiment_loops(
    db: &Option<Arc<dyn thinclaw::db::Database>>,
    config: &Config,
) -> Vec<RuntimeMaintenanceTask> {
    let mut tasks = Vec::new();
    if config.experiments.enabled {
        if let Some(db) = db.as_ref().cloned() {
            let experiments_db = Arc::clone(&db);
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            let handle = tokio::spawn(async move {
                thinclaw::api::experiments::start_experiment_controller_loop_with_shutdown(
                    experiments_db,
                    shutdown_rx,
                )
                .await;
            });
            tasks.push(RuntimeMaintenanceTask {
                name: "experiment_controller",
                shutdown_tx,
                handle,
            });
            tracing::info!("Experiment controller reconciler started (periodic cadence)");

            let reaper_db = Arc::clone(&db);
            let retention_days = config.experiments.default_artifact_retention_days;
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            let handle = tokio::spawn(async move {
                thinclaw::api::experiments::start_experiment_artifact_reaper_loop_with_shutdown(
                    reaper_db,
                    retention_days,
                    shutdown_rx,
                )
                .await;
            });
            tasks.push(RuntimeMaintenanceTask {
                name: "experiment_artifact_reaper",
                shutdown_tx,
                handle,
            });
            tracing::info!(
                "Experiment artifact reaper started (daily cadence, retention {retention_days}d)"
            );
        }
    } else {
        tracing::info!("Experiment controller not started because experiments are disabled.");
    }
    tasks
}
