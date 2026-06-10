#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStatus {
    Skipped,
    CircuitBreak,
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttemptTrace {
    pub attempt_no: u32,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub upstream_key_hint: Option<String>,
    pub requested_model: String,
    pub upstream_model: Option<String>,
    pub client_endpoint: Option<String>,
    pub upstream_endpoint: Option<String>,
    pub status: AttemptStatus,
    pub reason: Option<String>,
    pub duration_ms: Option<i64>,
    pub queue_wait_ms: Option<i64>,
    pub sticky: bool,
    pub score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct AttemptTraceBuilder {
    requested_model: String,
    traces: Vec<AttemptTrace>,
}

impl AttemptTraceBuilder {
    pub fn new(requested_model: impl Into<String>) -> Self {
        Self {
            requested_model: requested_model.into(),
            traces: Vec::new(),
        }
    }

    pub fn skipped(&mut self) -> AttemptTraceDraft<'_> {
        self.draft(AttemptStatus::Skipped)
    }

    pub fn circuit_break(&mut self) -> AttemptTraceDraft<'_> {
        self.draft(AttemptStatus::CircuitBreak)
    }

    pub fn success(&mut self) -> AttemptTraceDraft<'_> {
        self.draft(AttemptStatus::Success)
    }

    pub fn failed(&mut self) -> AttemptTraceDraft<'_> {
        self.draft(AttemptStatus::Failed)
    }

    pub fn finish_all(self) -> Vec<AttemptTrace> {
        self.traces
    }

    fn draft(&mut self, status: AttemptStatus) -> AttemptTraceDraft<'_> {
        let attempt = AttemptTrace {
            attempt_no: self.traces.len() as u32 + 1,
            channel_id: None,
            channel_name: None,
            upstream_key_hint: None,
            requested_model: self.requested_model.clone(),
            upstream_model: None,
            client_endpoint: None,
            upstream_endpoint: None,
            status,
            reason: None,
            duration_ms: None,
            queue_wait_ms: None,
            sticky: false,
            score: None,
        };
        AttemptTraceDraft {
            builder: self,
            attempt,
        }
    }
}

pub struct AttemptTraceDraft<'a> {
    builder: &'a mut AttemptTraceBuilder,
    attempt: AttemptTrace,
}

impl AttemptTraceDraft<'_> {
    pub fn channel(
        mut self,
        channel_id: impl Into<String>,
        channel_name: impl Into<String>,
    ) -> Self {
        self.attempt.channel_id = Some(channel_id.into());
        self.attempt.channel_name = Some(channel_name.into());
        self
    }

    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.attempt.reason = Some(reason.into());
        self
    }

    pub fn duration_ms(mut self, duration_ms: i64) -> Self {
        self.attempt.duration_ms = Some(duration_ms);
        self
    }

    pub fn sticky(mut self, sticky: bool) -> Self {
        self.attempt.sticky = sticky;
        self
    }

    pub fn score(mut self, score: f64) -> Self {
        self.attempt.score = Some(score);
        self
    }

    pub fn finish(self) {
        self.builder.traces.push(self.attempt);
    }
}
