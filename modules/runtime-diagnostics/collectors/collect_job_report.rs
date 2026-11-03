use crate::collectors::{
    mod_callbacks, operation_metadata, operation_timings, queue_health, runtime_health,
};
use crate::{Session, seconds};
use serde_json::{Value, json};

impl Session {
    pub(crate) fn sample(&mut self) {
        let mut report = json!({"jobId":self.job.as_ref().map(|job| job.id.as_str())});
        let runtime = self.includes_capability("runtime");
        let queue = self.includes_capability("queue");
        let is_operation_metadata = crate::request_diagnostic_action::is_operation_metadata_job_id(
            self.job.as_ref().map(|job| job.id.as_str()),
        );

        if is_operation_metadata {
            report["operationMetadata"] = operation_metadata::snapshot();
        } else if runtime || queue {
            let observed = runtime_health::snapshot();
            let mut native = if runtime {
                observed.clone()
            } else {
                queue_health::project(&observed)
            };
            if queue {
                queue_health::annotate_drain_age(&mut native);
            } else {
                native = runtime_health::without_queue_details(native);
            }
            report["native"] = native;
        }

        if mod_callbacks::enabled(self) {
            report["measurements"] = mod_callbacks::report(&self.measurements);
        }
        if self.includes_capability("operations") {
            report["operationTimings"] = operation_timings::snapshot();
        }
        if self.job.as_ref().is_some_and(|job| job.execution.is_some()) {
            report["execution"] = self.job_report.clone();
            report["exportsDirectory"] = json!(self.job_exports);
        }

        let mut signature = report.clone();
        queue_health::suppress_unchanging_counters(&mut signature);
        if self.settings["onlyChanges"] != true || signature != self.last_signature {
            let slow_callbacks = report
                .get("measurements")
                .is_some_and(mod_callbacks::failed);
            let operation_overflow = report
                .pointer("/operationTimings/droppedSpans")
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0);
            let provider_error = report
                .pointer("/native/error")
                .is_some_and(|error| !error.is_null());
            let execution_error = report.pointer("/execution/hostError").is_some()
                || report.pointer("/execution/cleanupError").is_some();
            if slow_callbacks || operation_overflow || provider_error || execution_error {
                tracing::warn!(target:"shroudforge::diagnostics",report=%report,"Diagnostic job sample");
            } else {
                tracing::debug!(target:"shroudforge::diagnostics",report=%report,"Diagnostic job sample");
            }
            self.last_signature = signature;
        }
        self.last_report = report;
        self.last_sample = seconds();
    }
}
