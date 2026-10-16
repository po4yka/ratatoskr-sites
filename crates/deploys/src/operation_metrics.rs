//! Fixed outcome counters. No request field becomes a metric label.
use crate::DeployError;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub(super) struct OperationMetrics {
    pub(super) verifications: Counts,
    pub(super) activations: Counts,
}

#[derive(Debug, Default)]
pub(super) struct Counts {
    success: AtomicU64,
    rejected: AtomicU64,
    busy: AtomicU64,
    unavailable: AtomicU64,
    timeout: AtomicU64,
    cancelled: AtomicU64,
}

impl Counts {
    pub(super) fn record<T>(&self, result: &Result<T, DeployError>, cancelled: bool) {
        let counter = match result {
            Ok(_) => &self.success,
            Err(_) if cancelled => &self.cancelled,
            Err(DeployError::Busy) => &self.busy,
            Err(DeployError::Timeout) => &self.timeout,
            Err(
                DeployError::VerifierUnavailable
                | DeployError::Storage(_)
                | DeployError::Database(_),
            ) => &self.unavailable,
            Err(_) => &self.rejected,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&self, name: &str) -> String {
        let mut output = format!("# TYPE {name} counter\n");
        for (outcome, counter) in [
            ("success", &self.success),
            ("rejected", &self.rejected),
            ("busy", &self.busy),
            ("unavailable", &self.unavailable),
            ("timeout", &self.timeout),
            ("cancelled", &self.cancelled),
        ] {
            let _ = writeln!(
                output,
                "{name}{{outcome=\"{outcome}\"}} {}",
                counter.load(Ordering::Relaxed)
            );
        }
        output
    }
}

impl OperationMetrics {
    pub(super) fn render(&self) -> String {
        self.verifications
            .render("ratatoskr_sites_bundle_verifications_total")
            + &self
                .activations
                .render("ratatoskr_sites_bundle_activations_total")
    }
}
