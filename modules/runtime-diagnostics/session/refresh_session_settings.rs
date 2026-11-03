use crate::Session;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) fn defaults() -> Value {
    json!({"enabled":false,"sessionActive":false,"continuous":true,"intervalMilliseconds":2000,"maximumDurationSeconds":120,"slowCallbackMilliseconds":20,"onlyChanges":true,"jobId":"runtime.overview","requestId":0})
}

impl Session {
    pub(super) fn refresh(&mut self) {
        if Instant::now() < self.next_config {
            return;
        }
        self.next_config = Instant::now() + Duration::from_millis(500);
        let config = match shroudforge_loader_config::config::read_loader(&self.root) {
            Ok(value) => value,
            Err(error) => {
                if error != self.last_error {
                    tracing::error!(target:"shroudforge::diagnostics","Configuration rejected: {error}");
                    self.last_error = error;
                }
                return;
            }
        };
        self.last_error.clear();
        let first_config_read = self.first_config_read;
        self.first_config_read = false;
        let mut next = defaults();
        if let Some(values) = config["modules"]["runtimeDiagnostics"].as_object() {
            for (key, value) in values {
                next[key] = value.clone();
            }
        }
        let stored_active = next["sessionActive"] == true;
        let active = next["enabled"] == true && next["sessionActive"] == true;
        let start = active
            && (self.settings["sessionActive"] != true
                || next["requestId"] != self.settings["requestId"]);

        let operation_metadata_job =
            crate::request_diagnostic_action::is_operation_metadata_job_id(next["jobId"].as_str());
        if operation_metadata_job && stored_active && (first_config_read || start) {
            let request_id = next["requestId"].as_u64();
            let fresh_snapshot = !first_config_read
                && active
                && next["continuous"] == false
                && request_id.is_some_and(|request_id| {
                    self.settings["requestId"].as_u64() != Some(request_id)
                });
            if !fresh_snapshot {
                if self.active {
                    self.stop_diagnostic_job();
                }
                if let Some(request_id) = request_id {
                    if let Err(error) =
                        crate::request_diagnostic_action::cancel_operation_metadata_request(
                            &self.root, request_id,
                        )
                    {
                        tracing::warn!(
                            target:"shroudforge::diagnostics",
                            %error,
                            "Could not cancel the stale operation metadata request"
                        );
                    }
                }
                self.active = false;
                self.reason = "stopped";
                self.settings = next;
                return;
            }
        }

        if !active && self.active {
            self.stop_diagnostic_job();
        }
        next["sessionActive"] = if operation_metadata_job {
            stored_active.into()
        } else {
            active.into()
        };
        self.settings = next;
        if start {
            self.start_diagnostic_job();
        }
    }
}
