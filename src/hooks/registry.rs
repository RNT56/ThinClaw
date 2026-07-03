//! Hook registry for managing and executing lifecycle hooks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use tokio::sync::RwLock;

use crate::hooks::hook::{Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome};

/// Number of consecutive failures/timeouts (under `FailOpen`) after which a
/// hook is automatically disabled (skipped on future events) rather than
/// silently eating its full timeout on every matching event forever.
///
/// Auto-disable applies to `FailOpen` hooks only: skipping a disabled hook
/// is equivalent to passing it, so disabling a `FailClosed` hook would
/// silently invert its guarantee into fail-open. `FailClosed` hooks keep
/// failing the chain on every event until they recover or are replaced;
/// their failure counter is still tracked for observability.
pub const MAX_CONSECUTIVE_HOOK_FAILURES: u32 = 5;

/// Serializable information about a registered hook.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HookInfo {
    pub name: String,
    pub hook_points: Vec<String>,
    pub failure_mode: String,
    pub timeout_ms: u64,
    pub priority: u32,
    /// `true` if the hook has been automatically disabled after
    /// [`MAX_CONSECUTIVE_HOOK_FAILURES`] consecutive failures/timeouts.
    pub disabled: bool,
    /// Current consecutive failure/timeout count (resets to 0 on success).
    pub consecutive_failures: u32,
}

/// A registered hook with its priority and health tracking.
struct HookEntry {
    hook: Arc<dyn Hook>,
    priority: u32,
    /// Consecutive failures/timeouts since the last success.
    consecutive_failures: AtomicU32,
    /// Set once `consecutive_failures` reaches [`MAX_CONSECUTIVE_HOOK_FAILURES`].
    /// A disabled hook is skipped by `HookRegistry::run` until manually
    /// re-enabled via [`HookRegistry::reenable`], re-registered under the
    /// same name (which resets health tracking), or the process restarts.
    disabled: AtomicBool,
}

impl HookEntry {
    fn new(hook: Arc<dyn Hook>, priority: u32) -> Self {
        Self {
            hook,
            priority,
            consecutive_failures: AtomicU32::new(0),
            disabled: AtomicBool::new(false),
        }
    }

    /// Record a successful execution, resetting the consecutive-failure
    /// counter. Does not clear `disabled` — a hook that reached the
    /// auto-disable threshold stays disabled (and therefore never runs
    /// again to naturally "recover") until explicitly re-enabled.
    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    /// Record a failure/timeout. Returns `true` if this call caused the
    /// hook to cross the auto-disable threshold (so the caller can log
    /// exactly once per disable event).
    ///
    /// Auto-disable is derived from the entry's own hook here — not passed
    /// by the caller — so no future failure path can forget the rule:
    /// skipping a disabled hook is equivalent to passing it, and a
    /// fail-closed hook must never silently convert into a pass-through; it
    /// keeps failing the chain on every event until it recovers or is
    /// replaced.
    fn record_failure(&self) -> bool {
        let previous = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        let count = previous + 1;
        let allow_disable = matches!(self.hook.failure_mode(), HookFailureMode::FailOpen);
        if allow_disable && count >= MAX_CONSECUTIVE_HOOK_FAILURES {
            !self.disabled.swap(true, Ordering::Relaxed)
        } else {
            false
        }
    }

    fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }
}

/// Registry that manages hooks and executes them at lifecycle points.
///
/// Hooks are executed in priority order (lower number = higher priority).
/// A `Reject` outcome stops the chain immediately.
/// A `Modify` outcome chains through subsequent hooks.
pub struct HookRegistry {
    hooks: RwLock<Vec<Arc<HookEntry>>>,
}

impl HookRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            hooks: RwLock::new(Vec::new()),
        }
    }

    /// Register a hook with default priority (100).
    pub async fn register(&self, hook: Arc<dyn Hook>) {
        self.register_with_priority(hook, 100).await;
    }

    /// Register a hook with a specific priority.
    ///
    /// Lower priority number = runs first.
    pub async fn register_with_priority(&self, hook: Arc<dyn Hook>, priority: u32) {
        let mut hooks = self.hooks.write().await;
        let hook_name = hook.name().to_string();

        if let Some(index) = hooks
            .iter()
            .position(|entry| entry.hook.name() == hook_name)
        {
            tracing::warn!(
                hook = %hook_name,
                "Replacing existing hook registration with same name"
            );
            // Replace the whole entry (not just `hook`/`priority`) so
            // re-registering a hook under the same name also resets its
            // health tracking — otherwise a hook that was fixed and
            // re-registered would stay disabled from its prior failure
            // streak. An invocation of the OLD hook still in flight records
            // its outcome on the orphaned entry (invisible here) — accepted:
            // fresh health state after replacement is the desired semantic.
            hooks[index] = Arc::new(HookEntry::new(hook, priority));
        } else {
            hooks.push(Arc::new(HookEntry::new(hook, priority)));
        }

        hooks.sort_by_key(|e| e.priority);
    }

    /// Manually re-enable a previously auto-disabled hook and reset its
    /// consecutive-failure counter.
    ///
    /// Returns `true` if a hook with this name was found (regardless of
    /// whether it was actually disabled).
    pub async fn reenable(&self, name: &str) -> bool {
        let hooks = self.hooks.read().await;
        if let Some(entry) = hooks.iter().find(|entry| entry.hook.name() == name) {
            entry.disabled.store(false, Ordering::Relaxed);
            entry.consecutive_failures.store(0, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Unregister a hook by name. Returns `true` if it was found and removed.
    pub async fn unregister(&self, name: &str) -> bool {
        let mut hooks = self.hooks.write().await;
        let before = hooks.len();
        hooks.retain(|e| e.hook.name() != name);
        hooks.len() < before
    }

    /// List all registered hook names (in priority order).
    pub async fn list(&self) -> Vec<String> {
        let hooks = self.hooks.read().await;
        hooks.iter().map(|e| e.hook.name().to_string()).collect()
    }

    /// List all registered hooks with detailed information.
    pub async fn list_with_details(&self) -> Vec<HookInfo> {
        let hooks = self.hooks.read().await;
        hooks
            .iter()
            .map(|e| HookInfo {
                name: e.hook.name().to_string(),
                hook_points: e
                    .hook
                    .hook_points()
                    .iter()
                    .map(|p| p.as_str().to_string())
                    .collect(),
                failure_mode: format!("{:?}", e.hook.failure_mode()),
                timeout_ms: e.hook.timeout().as_millis() as u64,
                priority: e.priority,
                disabled: e.is_disabled(),
                consecutive_failures: e.consecutive_failures.load(Ordering::Relaxed),
            })
            .collect()
    }

    /// Run all hooks matching the event's hook point, using a default
    /// (empty) [`HookContext`].
    ///
    /// - Hooks run in priority order (lowest first).
    /// - `Reject` stops the chain immediately.
    /// - `Modify` chains the modification through subsequent hooks.
    /// - Timeout/error handling respects each hook's `failure_mode`.
    /// - Hooks that have been auto-disabled after repeated consecutive
    ///   failures/timeouts are skipped (see [`MAX_CONSECUTIVE_HOOK_FAILURES`]).
    ///
    /// Callers that have real invocation metadata to pass to hooks should
    /// prefer [`HookRegistry::run_with_context`] — this method exists so
    /// call sites that only have an event (no context) keep compiling.
    ///
    /// Typed [`HookPatch`]es ARE applied along the chain here, but the
    /// patched event is discarded with only the outcome returned — a call
    /// site that needs to honor patched fields must use
    /// [`HookRegistry::run_returning_event`] instead (as the BeforeLlmInput
    /// dispatch site does).
    pub async fn run(&self, event: &HookEvent) -> Result<HookOutcome, HookError> {
        self.run_with_context(event, &HookContext::default()).await
    }

    /// Run all hooks matching the event's hook point, passing `ctx`
    /// (including its `metadata`) through to each hook's `execute`.
    ///
    /// Behaves identically to [`HookRegistry::run`] otherwise.
    pub async fn run_with_context(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        self.run_with_context_returning_event(event, ctx)
            .await
            .map(|(outcome, _event)| outcome)
    }

    /// Like [`HookRegistry::run_with_context`], but also returns the final
    /// event after all string modifications and typed [`HookPatch`]es were
    /// applied, so callers can honor patched fields (e.g. an `LlmInput`
    /// system-message override) that the string-diff `HookOutcome` cannot
    /// express.
    pub async fn run_returning_event(
        &self,
        event: &HookEvent,
    ) -> Result<(HookOutcome, HookEvent), HookError> {
        self.run_with_context_returning_event(event, &HookContext::default())
            .await
    }

    async fn run_with_context_returning_event(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
    ) -> Result<(HookOutcome, HookEvent), HookError> {
        let point = event.hook_point();

        // Clone matching, enabled hook entries and drop the read guard
        // before executing. Each hook can run up to its timeout, so
        // holding the guard would block concurrent register/unregister/run
        // calls. Cloning `Arc<HookEntry>` (rather than just `Arc<dyn Hook>`)
        // keeps each entry's health counters shared with the copy stored in
        // the registry, so failure/success tracking persists across calls.
        let matching: Vec<Arc<HookEntry>> = {
            let hooks = self.hooks.read().await;
            hooks
                .iter()
                .filter(|e| e.hook.hook_points().contains(&point))
                .filter_map(|e| {
                    if e.is_disabled() {
                        tracing::trace!(hook = e.hook.name(), "Skipping auto-disabled hook");
                        None
                    } else {
                        Some(e.clone())
                    }
                })
                .collect()
        };

        if matching.is_empty() {
            return Ok((HookOutcome::ok(), event.clone()));
        }

        let mut current_event = event.clone();

        for entry in &matching {
            let hook = &entry.hook;
            let timeout = hook.timeout();

            let result = tokio::time::timeout(timeout, hook.execute(&current_event, ctx)).await;

            match result {
                Ok(Ok(HookOutcome::Reject { reason })) => {
                    entry.record_success();
                    tracing::debug!(hook = hook.name(), "Hook rejected: {}", reason);
                    return Err(HookError::Rejected { reason });
                }
                Ok(Ok(HookOutcome::Continue {
                    modified: Some(value),
                })) => {
                    entry.record_success();
                    tracing::debug!(hook = hook.name(), "Hook modified content");
                    current_event.apply_modification(&value);
                    apply_hook_patch(hook.as_ref(), &mut current_event, ctx);
                }
                Ok(Ok(HookOutcome::Continue { modified: None })) => {
                    entry.record_success();
                    // No string modification; the hook may still return a
                    // typed patch.
                    apply_hook_patch(hook.as_ref(), &mut current_event, ctx);
                }
                Ok(Err(err)) => {
                    let just_disabled = entry.record_failure();
                    if just_disabled {
                        warn_hook_auto_disabled(hook.name());
                    }
                    match hook.failure_mode() {
                        HookFailureMode::FailOpen => {
                            tracing::warn!(hook = hook.name(), "Hook failed (fail-open): {}", err);
                        }
                        HookFailureMode::FailClosed => {
                            tracing::warn!(
                                hook = hook.name(),
                                "Hook failed (fail-closed): {}",
                                err
                            );
                            return Err(HookError::ExecutionFailed {
                                reason: format!("Hook '{}' failed: {}", hook.name(), err),
                            });
                        }
                    }
                }
                Err(_elapsed) => {
                    let just_disabled = entry.record_failure();
                    if just_disabled {
                        warn_hook_auto_disabled(hook.name());
                    }
                    match hook.failure_mode() {
                        HookFailureMode::FailOpen => {
                            tracing::warn!(
                                hook = hook.name(),
                                "Hook timed out (fail-open) after {:?}",
                                timeout
                            );
                        }
                        HookFailureMode::FailClosed => {
                            tracing::warn!(
                                hook = hook.name(),
                                "Hook timed out (fail-closed) after {:?}",
                                timeout
                            );
                            return Err(HookError::Timeout { timeout });
                        }
                    }
                }
            }
        }

        // Determine final outcome by comparing with original event
        let modified = extract_content(&current_event);
        let original = extract_content(event);

        let outcome = if modified != original {
            HookOutcome::modify(modified)
        } else {
            HookOutcome::ok()
        };
        Ok((outcome, current_event))
    }
}

/// Request a typed patch from a hook and apply it to the evolving event.
/// Runs synchronously after a successful `execute` — patch requests are
/// cheap by contract (no I/O) and share the hook's success accounting.
fn apply_hook_patch(hook: &dyn Hook, current_event: &mut HookEvent, ctx: &HookContext) {
    if let Some(patch) = hook.execute_patch(current_event, ctx) {
        tracing::debug!(hook = hook.name(), "Hook returned typed patch");
        patch.apply_to(current_event);
    }
}

/// Emit the warn log documenting that a hook was auto-disabled and how to
/// bring it back. Logged exactly once per disable transition (see
/// [`HookEntry::record_failure`]).
fn warn_hook_auto_disabled(hook_name: &str) {
    tracing::warn!(
        hook = hook_name,
        threshold = MAX_CONSECUTIVE_HOOK_FAILURES,
        "Hook auto-disabled after {} consecutive failures/timeouts and will be skipped on future \
         events. To re-enable it, call HookRegistry::reenable(\"{}\"), unregister and re-register \
         the hook under the same name (e.g. after fixing and reloading its bundle/config), or \
         restart the process to reload hook bundles from a fixed configuration.",
        MAX_CONSECUTIVE_HOOK_FAILURES,
        hook_name,
    );
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the primary content string from a hook event.
fn extract_content(event: &HookEvent) -> String {
    match event {
        HookEvent::Inbound { content, .. }
        | HookEvent::Outbound { content, .. }
        | HookEvent::MessageWrite { content, .. } => content.clone(),
        HookEvent::ToolCall { parameters, .. } => {
            serde_json::to_string(parameters).unwrap_or_default()
        }
        HookEvent::ResponseTransform { response, .. } => response.clone(),
        HookEvent::SessionStart { session_id, .. } | HookEvent::SessionEnd { session_id, .. } => {
            session_id.clone()
        }
        HookEvent::AgentStart { model, provider } => {
            format!("{}:{}", provider, model)
        }
        HookEvent::LlmInput { user_message, .. } => user_message.clone(),
        HookEvent::LlmOutput { content, .. } => content.clone(),
        HookEvent::TranscribeAudio { mime_type, .. } => mime_type.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::hook::{HookFailureMode, HookPoint};
    use async_trait::async_trait;
    use std::time::Duration;

    /// A test hook that always returns ok.
    struct PassthroughHook {
        name: String,
        points: Vec<HookPoint>,
    }

    #[async_trait]
    impl Hook for PassthroughHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            Ok(HookOutcome::ok())
        }
    }

    /// A hook that modifies content by appending a suffix.
    struct ModifyHook {
        name: String,
        suffix: String,
        points: Vec<HookPoint>,
    }

    #[async_trait]
    impl Hook for ModifyHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn execute(
            &self,
            event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            let content = extract_content(event);
            Ok(HookOutcome::modify(format!("{}{}", content, self.suffix)))
        }
    }

    /// A hook that returns a typed system-message patch for LlmInput.
    struct SystemPatchHook {
        name: String,
        system_message: String,
        points: Vec<HookPoint>,
    }

    #[async_trait]
    impl Hook for SystemPatchHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            Ok(HookOutcome::ok())
        }
        fn execute_patch(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Option<crate::hooks::HookPatch> {
            Some(crate::hooks::HookPatch::LlmInput {
                user_message: None,
                system_message: Some(self.system_message.clone()),
            })
        }
    }

    /// A hook that always rejects.
    struct RejectHook {
        name: String,
        reason: String,
        points: Vec<HookPoint>,
    }

    #[async_trait]
    impl Hook for RejectHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            Ok(HookOutcome::reject(&self.reason))
        }
    }

    /// A hook that always errors.
    struct ErrorHook {
        name: String,
        points: Vec<HookPoint>,
        failure_mode: HookFailureMode,
    }

    #[async_trait]
    impl Hook for ErrorHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        fn failure_mode(&self) -> HookFailureMode {
            self.failure_mode
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            Err(HookError::ExecutionFailed {
                reason: "test error".into(),
            })
        }
    }

    /// A hook that sleeps longer than its timeout.
    struct SlowHook {
        name: String,
        points: Vec<HookPoint>,
        failure_mode: HookFailureMode,
    }

    #[async_trait]
    impl Hook for SlowHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        fn failure_mode(&self) -> HookFailureMode {
            self.failure_mode
        }
        fn timeout(&self) -> Duration {
            Duration::from_millis(50)
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(HookOutcome::ok())
        }
    }

    /// A hook that fails its first `fail_count` calls, then succeeds on
    /// every call after that. Used to test auto-disable (crossing the
    /// threshold) and reset-on-success (recovering before the threshold).
    struct FlakyHook {
        name: String,
        points: Vec<HookPoint>,
        fail_count: usize,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Hook for FlakyHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        fn failure_mode(&self) -> HookFailureMode {
            HookFailureMode::FailOpen
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call < self.fail_count {
                Err(HookError::ExecutionFailed {
                    reason: format!("flaky failure #{}", call + 1),
                })
            } else {
                Ok(HookOutcome::ok())
            }
        }
    }

    /// A hook that always fails (fail-open) and records how many times it
    /// was actually invoked, to prove auto-disabled hooks stop being
    /// executed (not just stop affecting the outcome).
    struct AlwaysFailHook {
        name: String,
        points: Vec<HookPoint>,
        invocations: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl Hook for AlwaysFailHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        fn failure_mode(&self) -> HookFailureMode {
            HookFailureMode::FailOpen
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            _ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            self.invocations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(HookError::ExecutionFailed {
                reason: "always fails".into(),
            })
        }
    }

    /// A hook that records the metadata it was invoked with.
    struct MetadataCapturingHook {
        name: String,
        points: Vec<HookPoint>,
        captured: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    }

    #[async_trait]
    impl Hook for MetadataCapturingHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn hook_points(&self) -> &[HookPoint] {
            &self.points
        }
        async fn execute(
            &self,
            _event: &HookEvent,
            ctx: &HookContext,
        ) -> Result<HookOutcome, HookError> {
            self.captured
                .lock()
                .expect("captured lock")
                .push(ctx.metadata.clone());
            Ok(HookOutcome::ok())
        }
    }

    fn test_event() -> HookEvent {
        HookEvent::Inbound {
            user_id: "user-1".into(),
            channel: "test".into(),
            content: "hello".into(),
            thread_id: None,
        }
    }

    #[tokio::test]
    async fn test_empty_registry_returns_ok() {
        let registry = HookRegistry::new();
        let result = registry.run(&test_event()).await;
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap(),
            HookOutcome::Continue { modified: None }
        ));
    }

    #[tokio::test]
    async fn test_register_and_list() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(PassthroughHook {
                name: "hook-a".into(),
                points: vec![HookPoint::BeforeInbound],
            }))
            .await;
        registry
            .register(Arc::new(PassthroughHook {
                name: "hook-b".into(),
                points: vec![HookPoint::BeforeInbound],
            }))
            .await;

        let names = registry.list().await;
        assert_eq!(names, vec!["hook-a", "hook-b"]);
    }

    #[tokio::test]
    async fn test_register_duplicate_name_replaces_existing() {
        let registry = HookRegistry::new();

        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "dup".into(),
                    suffix: "-A".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                100,
            )
            .await;

        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "dup".into(),
                    suffix: "-B".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                10,
            )
            .await;

        let names = registry.list().await;
        assert_eq!(names, vec!["dup"]);

        let result = registry.run(&test_event()).await.unwrap();
        match result {
            HookOutcome::Continue {
                modified: Some(value),
            } => assert_eq!(value, "hello-B"),
            other => panic!("expected modified output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let registry = HookRegistry::new();

        // Register in reverse priority order
        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "low-prio".into(),
                    suffix: "-LOW".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                200,
            )
            .await;
        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "high-prio".into(),
                    suffix: "-HIGH".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                10,
            )
            .await;

        // Should run in priority order: high-prio first, then low-prio
        let names = registry.list().await;
        assert_eq!(names[0], "high-prio");
        assert_eq!(names[1], "low-prio");

        let result = registry.run(&test_event()).await.unwrap();
        match result {
            HookOutcome::Continue { modified: Some(m) } => {
                // "hello" -> "hello-HIGH" -> "hello-HIGH-LOW"
                assert_eq!(m, "hello-HIGH-LOW");
            }
            other => panic!("Expected modification chain, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_reject_stops_chain() {
        let registry = HookRegistry::new();

        registry
            .register_with_priority(
                Arc::new(RejectHook {
                    name: "blocker".into(),
                    reason: "blocked".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                10,
            )
            .await;
        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "modifier".into(),
                    suffix: "-MODIFIED".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                20,
            )
            .await;

        let result = registry.run(&test_event()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            HookError::Rejected { reason } => assert_eq!(reason, "blocked"),
            other => panic!("Expected Rejected, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_modification_chaining() {
        let registry = HookRegistry::new();

        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "first".into(),
                    suffix: "-A".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                10,
            )
            .await;
        registry
            .register_with_priority(
                Arc::new(ModifyHook {
                    name: "second".into(),
                    suffix: "-B".into(),
                    points: vec![HookPoint::BeforeInbound],
                }),
                20,
            )
            .await;

        let result = registry.run(&test_event()).await.unwrap();
        match result {
            HookOutcome::Continue { modified: Some(m) } => {
                assert_eq!(m, "hello-A-B");
            }
            other => panic!("Expected chained modification, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_fail_open_on_error() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(ErrorHook {
                name: "err-open".into(),
                points: vec![HookPoint::BeforeInbound],
                failure_mode: HookFailureMode::FailOpen,
            }))
            .await;

        let result = registry.run(&test_event()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fail_closed_on_error() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(ErrorHook {
                name: "err-closed".into(),
                points: vec![HookPoint::BeforeInbound],
                failure_mode: HookFailureMode::FailClosed,
            }))
            .await;

        let result = registry.run(&test_event()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HookError::ExecutionFailed { .. }
        ));
    }

    #[tokio::test]
    async fn test_fail_open_on_timeout() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(SlowHook {
                name: "slow-open".into(),
                points: vec![HookPoint::BeforeInbound],
                failure_mode: HookFailureMode::FailOpen,
            }))
            .await;

        let result = registry.run(&test_event()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_fail_closed_on_timeout() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(SlowHook {
                name: "slow-closed".into(),
                points: vec![HookPoint::BeforeInbound],
                failure_mode: HookFailureMode::FailClosed,
            }))
            .await;

        let result = registry.run(&test_event()).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HookError::Timeout { .. }));
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(PassthroughHook {
                name: "removable".into(),
                points: vec![HookPoint::BeforeInbound],
            }))
            .await;

        assert_eq!(registry.list().await.len(), 1);
        assert!(registry.unregister("removable").await);
        assert_eq!(registry.list().await.len(), 0);

        // Unregistering non-existent returns false
        assert!(!registry.unregister("nonexistent").await);
    }

    #[tokio::test]
    async fn test_hooks_only_match_their_points() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(RejectHook {
                name: "outbound-only".into(),
                reason: "blocked".into(),
                points: vec![HookPoint::BeforeOutbound],
            }))
            .await;

        // Inbound event should not be affected by outbound-only hook
        let result = registry.run(&test_event()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_auto_disables_after_max_consecutive_failures() {
        let registry = HookRegistry::new();
        let invocations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        registry
            .register(Arc::new(AlwaysFailHook {
                name: "always-fail".into(),
                points: vec![HookPoint::BeforeInbound],
                invocations: invocations.clone(),
            }))
            .await;

        // Fail-open, so `run` should keep reporting Ok even while the hook
        // is failing every call.
        for _ in 0..MAX_CONSECUTIVE_HOOK_FAILURES {
            let result = registry.run(&test_event()).await;
            assert!(result.is_ok());
        }
        assert_eq!(
            invocations.load(std::sync::atomic::Ordering::SeqCst),
            MAX_CONSECUTIVE_HOOK_FAILURES as usize
        );

        let details = registry.list_with_details().await;
        let info = details
            .iter()
            .find(|h| h.name == "always-fail")
            .expect("hook present");
        assert!(info.disabled, "hook should be auto-disabled");
        assert_eq!(info.consecutive_failures, MAX_CONSECUTIVE_HOOK_FAILURES);

        // Further events must not invoke the disabled hook at all — its
        // full timeout is no longer spent on every matching event.
        for _ in 0..3 {
            let result = registry.run(&test_event()).await;
            assert!(result.is_ok());
        }
        assert_eq!(
            invocations.load(std::sync::atomic::Ordering::SeqCst),
            MAX_CONSECUTIVE_HOOK_FAILURES as usize,
            "disabled hook must not run again"
        );
    }

    #[tokio::test]
    async fn test_success_resets_consecutive_failure_counter() {
        let registry = HookRegistry::new();
        // Fails one fewer time than the auto-disable threshold, then
        // succeeds — the counter should reset to 0 on that success instead
        // of carrying over toward the threshold.
        let fail_count = (MAX_CONSECUTIVE_HOOK_FAILURES - 1) as usize;
        registry
            .register(Arc::new(FlakyHook {
                name: "flaky".into(),
                points: vec![HookPoint::BeforeInbound],
                fail_count,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }))
            .await;

        for _ in 0..fail_count {
            let result = registry.run(&test_event()).await;
            assert!(result.is_ok());
        }

        let details = registry.list_with_details().await;
        let info = details
            .iter()
            .find(|h| h.name == "flaky")
            .expect("hook present");
        assert_eq!(info.consecutive_failures, fail_count as u32);
        assert!(!info.disabled);

        // Next call succeeds (call index == fail_count), resetting the
        // counter.
        let result = registry.run(&test_event()).await;
        assert!(result.is_ok());

        let details = registry.list_with_details().await;
        let info = details
            .iter()
            .find(|h| h.name == "flaky")
            .expect("hook present");
        assert_eq!(info.consecutive_failures, 0);
        assert!(!info.disabled);

        // The hook should now be able to accumulate a fresh run of
        // failures without being immediately disabled by leftover count.
        for _ in 0..fail_count {
            let result = registry.run(&test_event()).await;
            assert!(result.is_ok());
        }
        let details = registry.list_with_details().await;
        let info = details
            .iter()
            .find(|h| h.name == "flaky")
            .expect("hook present");
        assert!(!info.disabled, "should not be disabled prematurely");
    }

    #[tokio::test]
    async fn test_reenable_clears_disabled_state() {
        let registry = HookRegistry::new();
        let invocations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        registry
            .register(Arc::new(AlwaysFailHook {
                name: "always-fail".into(),
                points: vec![HookPoint::BeforeInbound],
                invocations: invocations.clone(),
            }))
            .await;

        for _ in 0..MAX_CONSECUTIVE_HOOK_FAILURES {
            registry.run(&test_event()).await.unwrap();
        }
        assert!(registry.list_with_details().await[0].disabled);

        assert!(registry.reenable("always-fail").await);
        let details = registry.list_with_details().await;
        assert!(!details[0].disabled);
        assert_eq!(details[0].consecutive_failures, 0);

        // Re-enabling a hook that isn't registered returns false.
        assert!(!registry.reenable("does-not-exist").await);
    }

    #[tokio::test]
    async fn test_run_with_context_passes_metadata_to_hooks() {
        let registry = HookRegistry::new();
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        registry
            .register(Arc::new(MetadataCapturingHook {
                name: "metadata-capture".into(),
                points: vec![HookPoint::BeforeInbound],
                captured: captured.clone(),
            }))
            .await;

        let metadata = serde_json::json!({"trace_id": "abc-123", "source": "test"});
        let ctx = HookContext {
            metadata: metadata.clone(),
        };
        let result = registry.run_with_context(&test_event(), &ctx).await;
        assert!(result.is_ok());

        let seen = captured.lock().expect("captured lock");
        assert_eq!(seen.as_slice(), &[metadata]);
    }

    #[tokio::test]
    async fn test_run_uses_default_empty_metadata() {
        // `run` (no explicit context) must still pass *some* HookContext
        // through — a default/empty one — rather than any stale value.
        let registry = HookRegistry::new();
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        registry
            .register(Arc::new(MetadataCapturingHook {
                name: "metadata-capture".into(),
                points: vec![HookPoint::BeforeInbound],
                captured: captured.clone(),
            }))
            .await;

        let result = registry.run(&test_event()).await;
        assert!(result.is_ok());

        let seen = captured.lock().expect("captured lock");
        assert_eq!(seen.as_slice(), &[serde_json::Value::Null]);
    }

    #[tokio::test]
    async fn fail_closed_hook_never_auto_disables_into_pass_through() {
        struct AlwaysFailingClosedHook;
        #[async_trait]
        impl Hook for AlwaysFailingClosedHook {
            fn name(&self) -> &str {
                "guardrail"
            }
            fn hook_points(&self) -> &[HookPoint] {
                &[HookPoint::BeforeToolCall]
            }
            fn failure_mode(&self) -> HookFailureMode {
                HookFailureMode::FailClosed
            }
            async fn execute(
                &self,
                _event: &HookEvent,
                _ctx: &HookContext,
            ) -> Result<HookOutcome, HookError> {
                Err(HookError::ExecutionFailed {
                    reason: "backend unreachable".to_string(),
                })
            }
        }

        let registry = HookRegistry::new();
        registry.register(Arc::new(AlwaysFailingClosedHook)).await;

        let event = HookEvent::ToolCall {
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({}),
            user_id: "user".to_string(),
            context: "chat".to_string(),
        };

        // Far past MAX_CONSECUTIVE_HOOK_FAILURES: every call must still fail
        // the chain. A fail-closed guardrail must never silently become a
        // pass-through.
        for _ in 0..(MAX_CONSECUTIVE_HOOK_FAILURES + 3) {
            let result = registry.run(&event).await;
            assert!(result.is_err(), "FailClosed hook must keep blocking");
        }
    }

    #[tokio::test]
    async fn typed_patch_reaches_the_returned_event() {
        let registry = HookRegistry::new();
        registry
            .register(Arc::new(SystemPatchHook {
                name: "system-patcher".to_string(),
                system_message: "patched system prompt".to_string(),
                points: vec![HookPoint::BeforeLlmInput],
            }))
            .await;

        let event = HookEvent::LlmInput {
            model: "test-model".to_string(),
            system_message: Some("original system prompt".to_string()),
            user_message: "hello".to_string(),
            message_count: 2,
            user_id: "user".to_string(),
        };

        let (outcome, final_event) = registry
            .run_returning_event(&event)
            .await
            .expect("hook chain succeeds");

        // The patch does not change the string-diff outcome (user message
        // untouched), but the returned event carries the override.
        assert!(matches!(outcome, HookOutcome::Continue { modified: None }));
        match final_event {
            HookEvent::LlmInput {
                system_message,
                user_message,
                ..
            } => {
                assert_eq!(system_message.as_deref(), Some("patched system prompt"));
                assert_eq!(user_message, "hello");
            }
            other => panic!("unexpected event variant: {other:?}"),
        }
    }
}
