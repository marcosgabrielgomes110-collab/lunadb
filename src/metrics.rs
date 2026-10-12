use std::sync::atomic::{AtomicU64, Ordering};

/// Low-cost counters exposed on `/metrics` (Prometheus format).
#[derive(Debug, Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub responses_2xx: AtomicU64,
    pub responses_3xx: AtomicU64,
    pub responses_4xx: AtomicU64,
    pub responses_5xx: AtomicU64,
}

impl Metrics {
    pub fn record(&self, status: u16) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let bucket = match status {
            200..=299 => &self.responses_2xx,
            300..=399 => &self.responses_3xx,
            400..=499 => &self.responses_4xx,
            _ => &self.responses_5xx,
        };
        bucket.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self, version: &str, uptime_seconds: i64) -> String {
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        format!(
            "# HELP lunadb_build_info LunaDB build information\n\
             # TYPE lunadb_build_info gauge\n\
             lunadb_build_info{{version=\"{version}\"}} 1\n\
             # HELP lunadb_uptime_seconds Uptime in seconds\n\
             # TYPE lunadb_uptime_seconds gauge\n\
             lunadb_uptime_seconds {uptime_seconds}\n\
             # HELP lunadb_requests_total Total HTTP requests\n\
             # TYPE lunadb_requests_total counter\n\
             lunadb_requests_total {}\n\
             # HELP lunadb_responses_total HTTP responses by status class\n\
             # TYPE lunadb_responses_total counter\n\
             lunadb_responses_total{{status_class=\"2xx\"}} {}\n\
             lunadb_responses_total{{status_class=\"3xx\"}} {}\n\
             lunadb_responses_total{{status_class=\"4xx\"}} {}\n\
             lunadb_responses_total{{status_class=\"5xx\"}} {}\n",
            load(&self.requests),
            load(&self.responses_2xx),
            load(&self.responses_3xx),
            load(&self.responses_4xx),
            load(&self.responses_5xx),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_by_status_class() {
        let metrics = Metrics::default();
        metrics.record(200);
        metrics.record(404);
        metrics.record(500);
        assert_eq!(metrics.requests.load(Ordering::Relaxed), 3);
        assert_eq!(metrics.responses_2xx.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.responses_4xx.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.responses_5xx.load(Ordering::Relaxed), 1);
        let text = metrics.render("0.0.1", 5);
        assert!(text.contains("lunadb_requests_total 3"));
        assert!(text.contains("lunadb_uptime_seconds 5"));
    }
}
