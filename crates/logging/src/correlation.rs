use tracing::Span;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::{LookupSpan, SpanRef};

/// Creates a span that opts into the shared correlation-field conventions.
pub fn runtime_span(name: &'static str) -> Span {
    tracing::span!(
        tracing::Level::INFO,
        "runtime",
        span = name,
        trace_id = tracing::field::Empty,
        request_id = tracing::field::Empty
    )
}

/// Creates a span with a stable top-level `trace_id` value for downstream events.
pub fn span_with_trace_id(name: &'static str, trace_id: &str) -> Span {
    span_with_correlation(name, Some(trace_id), None)
}

/// Creates a span with a stable top-level `request_id` value for downstream events.
pub fn span_with_request_id(name: &'static str, request_id: &str) -> Span {
    span_with_correlation(name, None, Some(request_id))
}

/// Creates a span that attaches the optional shared correlation fields consistently.
pub fn span_with_correlation(
    name: &'static str,
    trace_id: Option<&str>,
    request_id: Option<&str>,
) -> Span {
    let span = runtime_span(name);

    if let Some(trace_id) = trace_id {
        span.record("trace_id", trace_id);
    }
    if let Some(request_id) = request_id {
        span.record("request_id", request_id);
    }

    span
}

/// Stores span-scoped correlation values so the formatter can emit them at the top level.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CorrelationState {
    pub(crate) name: Option<String>,
    pub(crate) trace_id: Option<String>,
    pub(crate) request_id: Option<String>,
}

/// Captures correlation fields recorded on spans and exposes them through span extensions.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CorrelationLayer;

impl<S> Layer<S> for CorrelationLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    /// Persists the current span name plus any optional correlation fields when a span is created.
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        context: Context<'_, S>,
    ) {
        let Some(span) = context.span(id) else {
            return;
        };

        let mut state = CorrelationState {
            name: Some(span.metadata().name().to_string()),
            ..CorrelationState::default()
        };
        let mut visitor = CorrelationVisitor::new(&mut state);
        attributes.record(&mut visitor);
        span.extensions_mut().insert(state);
    }

    /// Updates the stored correlation fields when code records new values onto an existing span.
    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        context: Context<'_, S>,
    ) {
        let Some(span) = context.span(id) else {
            return;
        };

        let mut extensions = span.extensions_mut();
        if let Some(state) = extensions.get_mut::<CorrelationState>() {
            let mut visitor = CorrelationVisitor::new(state);
            values.record(&mut visitor);
        }
    }
}

/// Returns the active span-scoped correlation fields visible to one event.
pub(crate) fn scope_correlation<S>(
    scope: Option<tracing_subscriber::registry::Scope<'_, S>>,
) -> CorrelationState
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    let mut correlation = CorrelationState::default();

    if let Some(scope) = scope {
        for span in scope.from_root() {
            merge_correlation_from_span(span, &mut correlation);
        }
    }

    correlation
}

/// Merges a span's correlation state so the innermost explicit values win naturally.
fn merge_correlation_from_span<S>(span: SpanRef<'_, S>, correlation: &mut CorrelationState)
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    let extensions = span.extensions();
    let Some(state) = extensions.get::<CorrelationState>() else {
        return;
    };

    if let Some(name) = &state.name {
        correlation.name = Some(name.clone());
    }
    if let Some(trace_id) = &state.trace_id {
        correlation.trace_id = Some(trace_id.clone());
    }
    if let Some(request_id) = &state.request_id {
        correlation.request_id = Some(request_id.clone());
    }
}

/// Records only the fields that belong to the shared correlation envelope.
struct CorrelationVisitor<'state> {
    state: &'state mut CorrelationState,
}

impl<'state> CorrelationVisitor<'state> {
    /// Builds the visitor around mutable span-scoped correlation storage.
    fn new(state: &'state mut CorrelationState) -> Self {
        Self { state }
    }
}

impl tracing::field::Visit for CorrelationVisitor<'_> {
    /// Preserves string correlation values exactly as runtime code provided them.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "span" => {
                self.state.name = Some(value.to_string());
            }
            "trace_id" => {
                self.state.trace_id = Some(value.to_string());
            }
            "request_id" => {
                self.state.request_id = Some(value.to_string());
            }
            _ => {}
        }
    }

    /// Falls back to debug formatting so non-string correlation values still become usable strings.
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "span" => {
                self.state.name = Some(format!("{value:?}").trim_matches('"').to_string());
            }
            "trace_id" => {
                self.state.trace_id = Some(format!("{value:?}").trim_matches('"').to_string());
            }
            "request_id" => {
                self.state.request_id = Some(format!("{value:?}").trim_matches('"').to_string());
            }
            _ => {}
        }
    }
}
