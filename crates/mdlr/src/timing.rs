use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::span;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::registry::LookupSpan;

struct SpanTiming {
    start: Instant,
}

pub struct TimingLayer {
    timings: Arc<Mutex<Vec<(String, std::time::Duration)>>>,
}

impl TimingLayer {
    pub fn new() -> (Self, TimingPrinter) {
        let timings = Arc::new(Mutex::new(Vec::new()));
        let printer = TimingPrinter { timings: timings.clone() };
        (Self { timings }, printer)
    }
}

impl<S> Layer<S> for TimingLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        _attrs: &span::Attributes<'_>,
        id: &span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanTiming { start: Instant::now() });
        }
    }

    fn on_close(
        &self,
        id: span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if let Some(span) = ctx.span(&id) {
            let ext = span.extensions();
            if let Some(timing) = ext.get::<SpanTiming>() {
                let duration = timing.start.elapsed();
                let name = span.name().to_string();
                self.timings.lock().unwrap().push((name, duration));
            }
        }
    }
}

pub struct TimingPrinter {
    timings: Arc<Mutex<Vec<(String, std::time::Duration)>>>,
}

impl TimingPrinter {
    pub fn print(&self) {
        let timings = self.timings.lock().unwrap();
        if timings.is_empty() {
            return;
        }
        eprintln!();
        eprintln!("timing:");
        for (name, duration) in timings.iter() {
            let secs = duration.as_secs_f64();
            if secs >= 1.0 {
                eprintln!("  {:<24} {:.2}s", name, secs);
            } else {
                eprintln!("  {:<24} {:.0}ms", name, secs * 1000.0);
            }
        }
    }
}
