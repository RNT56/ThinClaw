use chrono::Utc;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use thinclaw_agent::loop_control::{LoopBudget, LoopKind, LoopRunContext};
use thinclaw_agent::routine::{RoutineEvent, RoutineEventStatus};
use thinclaw_agent::routine_engine::fair_interleave_routine_events;
use uuid::Uuid;

fn routine_event(source: usize, sequence: usize) -> RoutineEvent {
    RoutineEvent {
        id: Uuid::from_u128(sequence as u128 + 1),
        principal_id: "bench".to_string(),
        actor_id: "agent".to_string(),
        channel: "benchmark".to_string(),
        event_type: "message".to_string(),
        raw_sender_id: source.to_string(),
        conversation_scope_id: source.to_string(),
        stable_external_conversation_key: format!("source://{source}"),
        idempotency_key: format!("{source}:{sequence}"),
        content: sequence.to_string(),
        content_hash: sequence.to_string(),
        metadata: serde_json::json!({}),
        status: RoutineEventStatus::Pending,
        diagnostics: serde_json::json!({}),
        claimed_by: None,
        claimed_at: None,
        lease_expires_at: None,
        processed_at: None,
        error_message: None,
        matched_routines: 0,
        fired_routines: 0,
        attempt_count: 0,
        created_at: Utc::now(),
    }
}

fn bench_loop_control(c: &mut Criterion) {
    c.bench_function("loop_budget_10k_iterations", |b| {
        b.iter(|| {
            let mut context = LoopRunContext::new(LoopKind::Worker, LoopBudget::iterations(10_000));
            for _ in 0..10_000 {
                black_box(
                    context
                        .begin_iteration()
                        .expect("iteration should fit budget"),
                );
            }
        });
    });

    c.bench_function("routine_fair_interleave_4096_events_64_sources", |b| {
        b.iter_batched(
            || {
                (0..4096)
                    .map(|sequence| routine_event(sequence % 64, sequence))
                    .collect::<Vec<_>>()
            },
            |events| black_box(fair_interleave_routine_events(events)),
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, bench_loop_control);
criterion_main!(benches);
