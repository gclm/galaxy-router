use galaxy_router::scheduler::trace::{AttemptStatus, AttemptTraceBuilder};

#[test]
fn scheduler_trace_builder_records_skip_circuit_success_and_failure_sequence() {
    let mut builder = AttemptTraceBuilder::new("gpt-4o");

    builder
        .skipped()
        .channel("ch-a", "primary")
        .reason("model not allowed")
        .score(2.78)
        .finish();

    builder
        .circuit_break()
        .channel("ch-b", "backup")
        .reason("circuit open")
        .finish();

    builder
        .success()
        .channel("ch-c", "fast")
        .duration_ms(128)
        .sticky(true)
        .finish();

    builder
        .failed()
        .channel("ch-d", "slow")
        .reason("upstream 429")
        .duration_ms(42)
        .finish();

    let traces = builder.finish_all();

    assert_eq!(traces.len(), 4);
    assert_eq!(traces[0].attempt_no, 1);
    assert_eq!(traces[0].status, AttemptStatus::Skipped);
    assert_eq!(traces[0].channel_id.as_deref(), Some("ch-a"));
    assert_eq!(traces[0].channel_name.as_deref(), Some("primary"));
    assert_eq!(traces[0].requested_model, "gpt-4o");
    assert_eq!(traces[0].reason.as_deref(), Some("model not allowed"));
    assert_eq!(traces[0].score, Some(2.78));

    assert_eq!(traces[1].attempt_no, 2);
    assert_eq!(traces[1].status, AttemptStatus::CircuitBreak);
    assert_eq!(traces[1].reason.as_deref(), Some("circuit open"));

    assert_eq!(traces[2].attempt_no, 3);
    assert_eq!(traces[2].status, AttemptStatus::Success);
    assert_eq!(traces[2].duration_ms, Some(128));
    assert!(traces[2].sticky);

    assert_eq!(traces[3].attempt_no, 4);
    assert_eq!(traces[3].status, AttemptStatus::Failed);
    assert_eq!(traces[3].reason.as_deref(), Some("upstream 429"));
    assert_eq!(traces[3].duration_ms, Some(42));
}

#[test]
fn scheduler_trace_builder_serializes_status_as_stable_lowercase_strings() {
    let mut builder = AttemptTraceBuilder::new("gpt-4o-mini");
    builder.success().channel("ch-a", "primary").finish();

    let value = serde_json::to_value(builder.finish_all()).expect("serializable traces");

    assert_eq!(value[0]["status"], "success");
    assert_eq!(value[0]["attempt_no"], 1);
    assert_eq!(value[0]["requested_model"], "gpt-4o-mini");
}
