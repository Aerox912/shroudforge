use serde::Deserialize;
use serde_json::{Value, json};

const MAX_METADATA_BYTES: usize = 64 * 1024;
const CONTRACT_ID: &str = "kfc-runtime.operation-metadata.v1";

#[cfg(test)]
thread_local! {
    static SNAPSHOT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_snapshot_calls() {
    SNAPSHOT_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn snapshot_calls() -> usize {
    SNAPSHOT_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn snapshot() -> Value {
    SNAPSHOT_CALLS.with(|calls| calls.set(calls.get() + 1));
    json!({
        "contractId": CONTRACT_ID,
        "resolutionStatus": "test-stub"
    })
}

#[cfg(all(windows, not(test)))]
pub(super) fn snapshot() -> Value {
    native_snapshot()
}

#[cfg(all(not(windows), not(test)))]
pub(super) fn snapshot() -> Value {
    json!({"resolutionStatus":"native-provider-requires-windows"})
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeMetadata {
    contract_id: String,
    resolution_status: String,
    #[serde(default, deserialize_with = "deserialize_present")]
    runtime_abi: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present")]
    profile: Option<NativeProfile>,
    #[serde(default, deserialize_with = "deserialize_present")]
    observed_image: Option<ObservedImage>,
    #[serde(default, deserialize_with = "deserialize_present")]
    session_generation: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_present")]
    operation: Option<NativeOperation>,
    #[serde(default, deserialize_with = "deserialize_present")]
    writer_status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present")]
    writer_candidate_rva: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present")]
    writer_path: Option<Vec<WriterPathStep>>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeProfile {
    id: String,
    target: String,
    exact_image_match: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObservedImage {
    sha256: String,
    timestamp: u32,
    size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeOperation {
    name: String,
    argument_count: u8,
    parameter_schema: String,
    callback_rva: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WriterPathStep {
    rva: u32,
    code_sha256: String,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn has_envelope(metadata: &NativeMetadata) -> bool {
    let present = [
        metadata.runtime_abi.is_some(),
        metadata.profile.is_some(),
        metadata.observed_image.is_some(),
    ];
    present.into_iter().all(|value| value) || present.into_iter().all(|value| !value)
}

fn validate_envelope(metadata: &NativeMetadata) -> bool {
    if !has_envelope(metadata) {
        return false;
    }
    let (Some(runtime_abi), Some(profile), Some(image)) = (
        metadata.runtime_abi,
        metadata.profile.as_ref(),
        metadata.observed_image.as_ref(),
    ) else {
        return true;
    };
    runtime_abi > 0
        && profile.id == "not-retained"
        && matches!(
            profile.target.as_str(),
            "enshrouded.exe" | "enshrouded_server.exe" | "unknown"
        )
        && (image.sha256 == "unknown" || is_sha256(&image.sha256))
}

fn validate_operation(metadata: &NativeMetadata) -> bool {
    let (Some(generation), Some(operation), Some(image)) = (
        metadata.session_generation,
        metadata.operation.as_ref(),
        metadata.observed_image.as_ref(),
    ) else {
        return false;
    };
    generation > 0
        && image.size > 0
        && operation.name == "terraforming"
        && operation.parameter_schema == "unknown"
        && operation.callback_rva != 0
        && operation.callback_rva < image.size
}

fn valid_partial_writer_status(metadata: &NativeMetadata) -> bool {
    let Some(operation) = metadata.operation.as_ref() else {
        return false;
    };
    match metadata.writer_status.as_deref() {
        Some("unsupported-arity") => operation.argument_count != 9,
        Some("busy" | "decode-failure" | "invalid-metadata") => operation.argument_count == 9,
        _ => false,
    }
}

fn validate_writer_path(metadata: &NativeMetadata) -> bool {
    let (Some(operation), Some(image), Some(writer_rva), Some(path)) = (
        metadata.operation.as_ref(),
        metadata.observed_image.as_ref(),
        metadata.writer_candidate_rva,
        metadata.writer_path.as_ref(),
    ) else {
        return false;
    };
    if writer_rva == 0 || writer_rva >= image.size || path.is_empty() || path.len() > 7 {
        return false;
    }
    if path
        .first()
        .is_none_or(|step| step.rva != operation.callback_rva)
        || path.last().is_none_or(|step| step.rva != writer_rva)
    {
        return false;
    }
    for (index, step) in path.iter().enumerate() {
        if step.rva >= image.size
            || !is_sha256(&step.code_sha256)
            || path[..index]
                .iter()
                .any(|previous| previous.rva == step.rva)
        {
            return false;
        }
    }
    true
}

fn validate(metadata: &NativeMetadata) -> bool {
    if metadata.contract_id != CONTRACT_ID || !validate_envelope(metadata) {
        return false;
    }

    let details_present = metadata.session_generation.is_some()
        || metadata.operation.is_some()
        || metadata.writer_status.is_some()
        || metadata.writer_candidate_rva.is_some()
        || metadata.writer_path.is_some();
    match metadata.resolution_status.as_str() {
        "output-too-small" => {
            !metadata.runtime_abi.is_some()
                && metadata.profile.is_none()
                && metadata.observed_image.is_none()
                && !details_present
        }
        "unknown" | "busy" | "stale" | "unsupported" | "decode-failure" => !details_present,
        "partial" => {
            metadata.runtime_abi.is_some()
                && metadata.profile.is_some()
                && metadata.observed_image.is_some()
                && validate_operation(metadata)
                && valid_partial_writer_status(metadata)
                && metadata.writer_candidate_rva.is_none()
                && metadata.writer_path.is_none()
        }
        "found" => {
            metadata.runtime_abi.is_some()
                && metadata.profile.is_some()
                && metadata.observed_image.is_some()
                && validate_operation(metadata)
                && metadata
                    .operation
                    .as_ref()
                    .is_some_and(|operation| operation.argument_count == 9)
                && metadata.writer_status.as_deref() == Some("found")
                && validate_writer_path(metadata)
        }
        _ => false,
    }
}

fn sanitize_native_report(bytes: &[u8]) -> Option<Value> {
    if bytes.len() >= MAX_METADATA_BYTES {
        return None;
    }
    let metadata: NativeMetadata = serde_json::from_slice(bytes).ok()?;
    if !validate(&metadata) {
        return None;
    }

    let mut report = json!({
        "contractId": metadata.contract_id,
        "resolutionStatus": metadata.resolution_status,
    });
    if let (Some(runtime_abi), Some(profile), Some(image)) = (
        metadata.runtime_abi,
        metadata.profile,
        metadata.observed_image,
    ) {
        report["runtimeAbi"] = json!(runtime_abi);
        report["profile"] = json!({
            "id": profile.id,
            "target": profile.target,
            "exactImageMatch": profile.exact_image_match,
        });
        report["observedImage"] = json!({
            "sha256": image.sha256,
            "timestamp": image.timestamp,
            "size": image.size,
        });
    }
    if let (Some(generation), Some(operation), Some(writer_status)) = (
        metadata.session_generation,
        metadata.operation,
        metadata.writer_status,
    ) {
        report["sessionGeneration"] = json!(generation);
        report["operation"] = json!({
            "name": operation.name,
            "argumentCount": operation.argument_count,
            "parameterSchema": operation.parameter_schema,
            "callbackRva": operation.callback_rva,
        });
        report["writerStatus"] = json!(writer_status);
    }
    if let (Some(writer_rva), Some(path)) = (metadata.writer_candidate_rva, metadata.writer_path) {
        report["writerCandidateRva"] = json!(writer_rva);
        report["writerPath"] = path
            .into_iter()
            .map(|step| json!({"rva":step.rva,"codeSha256":step.code_sha256}))
            .collect::<Vec<_>>()
            .into();
    }
    Some(report)
}

fn invalid_provider_report() -> Value {
    json!({"resolutionStatus":"provider-report-invalid"})
}

#[cfg(all(windows, not(test)))]
fn native_snapshot() -> Value {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    let module_name: Vec<u16> = "kfc-runtime.dll\0".encode_utf16().collect();
    unsafe {
        let module = GetModuleHandleW(module_name.as_ptr());
        if module.is_null() {
            return json!({"resolutionStatus":"runtime-provider-not-loaded"});
        }
        let Some(symbol) = GetProcAddress(module, c"KfcRuntimeOperationMetadata".as_ptr().cast())
        else {
            return json!({"resolutionStatus":"operation-metadata-export-unavailable"});
        };
        let function: unsafe extern "C" fn(*mut u8, usize) = std::mem::transmute(symbol);
        let mut buffer = vec![0u8; MAX_METADATA_BYTES];
        function(buffer.as_mut_ptr(), buffer.len());
        let Some(length) = buffer.iter().position(|byte| *byte == 0) else {
            return invalid_provider_report();
        };
        sanitize_native_report(&buffer[..length]).unwrap_or_else(invalid_provider_report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found_report() -> Value {
        json!({
            "contractId": CONTRACT_ID,
            "resolutionStatus": "found",
            "runtimeAbi": 4,
            "profile": {
                "id": "not-retained",
                "target": "enshrouded.exe",
                "exactImageMatch": true
            },
            "observedImage": {
                "sha256": "a".repeat(64),
                "timestamp": 123,
                "size": 4096
            },
            "sessionGeneration": 7,
            "operation": {
                "name": "terraforming",
                "argumentCount": 9,
                "parameterSchema": "unknown",
                "callbackRva": 256
            },
            "writerStatus": "found",
            "writerCandidateRva": 512,
            "writerPath": [
                {"rva": 256, "codeSha256": "b".repeat(64)},
                {"rva": 512, "codeSha256": "c".repeat(64)}
            ]
        })
    }

    fn partial_report(argument_count: u8, writer_status: &str) -> Value {
        let mut report = found_report();
        report["resolutionStatus"] = json!("partial");
        report["operation"]["argumentCount"] = json!(argument_count);
        report["writerStatus"] = json!(writer_status);
        report.as_object_mut().unwrap().remove("writerCandidateRva");
        report.as_object_mut().unwrap().remove("writerPath");
        report
    }

    fn encoded(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    #[test]
    fn accepts_only_the_bounded_allowlisted_metadata_shape() {
        let output = sanitize_native_report(&encoded(&found_report())).unwrap();
        assert_eq!(output["operation"]["name"], "terraforming");
        assert_eq!(output["operation"]["parameterSchema"], "unknown");
        assert_eq!(output["operation"]["callbackRva"], 256);
        assert_eq!(output["writerCandidateRva"], 512);
        assert_eq!(output["writerPath"].as_array().unwrap().len(), 2);
        assert!(output["writerPath"][0].get("code").is_none());
    }

    #[test]
    fn accepts_minimal_native_unavailable_responses() {
        for status in [
            "unknown",
            "busy",
            "stale",
            "unsupported",
            "decode-failure",
            "output-too-small",
        ] {
            let bytes = encoded(&json!({
                "contractId": CONTRACT_ID,
                "resolutionStatus": status
            }));
            let output = sanitize_native_report(&bytes).unwrap();
            assert_eq!(output["resolutionStatus"], status);
            assert_eq!(output.as_object().unwrap().len(), 2);
        }
    }

    #[test]
    fn accepts_partial_metadata_without_a_writer_path() {
        let partial = partial_report(8, "unsupported-arity");

        let output = sanitize_native_report(&encoded(&partial)).unwrap();
        assert_eq!(output["resolutionStatus"], "partial");
        assert_eq!(output["writerStatus"], "unsupported-arity");
        assert!(output.get("writerPath").is_none());
    }

    #[test]
    fn status_arity_combinations_match_native_writer_discovery() {
        let accepted = [
            (
                "unsupported arity eight",
                partial_report(8, "unsupported-arity"),
            ),
            (
                "unsupported arity ten",
                partial_report(10, "unsupported-arity"),
            ),
            ("busy writer at arity nine", partial_report(9, "busy")),
            (
                "decode failure at arity nine",
                partial_report(9, "decode-failure"),
            ),
            (
                "invalid metadata at arity nine",
                partial_report(9, "invalid-metadata"),
            ),
            ("found writer at arity nine", found_report()),
        ];
        for (case, report) in accepted {
            assert!(
                sanitize_native_report(&encoded(&report)).is_some(),
                "native case was rejected: {case}"
            );
        }

        let mut found_at_eight = found_report();
        found_at_eight["operation"]["argumentCount"] = json!(8);
        let mut zero_callback_rva = found_report();
        zero_callback_rva["operation"]["callbackRva"] = json!(0);
        let rejected = [
            (
                "unsupported arity at nine",
                partial_report(9, "unsupported-arity"),
            ),
            ("busy writer at arity eight", partial_report(8, "busy")),
            (
                "decode failure at arity eight",
                partial_report(8, "decode-failure"),
            ),
            (
                "invalid metadata at arity eight",
                partial_report(8, "invalid-metadata"),
            ),
            (
                "unproduced not-inspected status",
                partial_report(8, "not-inspected"),
            ),
            ("unproduced writer stale status", partial_report(9, "stale")),
            ("found writer at arity eight", found_at_eight),
            ("zero callback RVA", zero_callback_rva),
        ];
        for (case, report) in rejected {
            assert!(
                sanitize_native_report(&encoded(&report)).is_none(),
                "impossible native case was accepted: {case}"
            );
        }
    }

    #[test]
    fn accepts_native_top_level_stale_envelope_without_operation_details() {
        let mut stale = found_report();
        stale["resolutionStatus"] = json!("stale");
        for field in [
            "sessionGeneration",
            "operation",
            "writerStatus",
            "writerCandidateRva",
            "writerPath",
        ] {
            stale.as_object_mut().unwrap().remove(field);
        }

        let output = sanitize_native_report(&encoded(&stale)).unwrap();
        assert_eq!(output["resolutionStatus"], "stale");
        assert!(output.get("observedImage").is_some());
        assert!(output.get("operation").is_none());
        assert!(output.get("sessionGeneration").is_none());
        assert!(output.get("writerStatus").is_none());
    }

    #[test]
    fn rejects_unknown_raw_and_malformed_provider_fields_without_echoing_them() {
        let mut invalid_reports = Vec::new();

        let mut raw_address = found_report();
        raw_address["scenePointer"] = json!("0x12345678");
        invalid_reports.push(raw_address);

        let mut unsafe_target = found_report();
        unsafe_target["profile"]["target"] = json!("C:\\private\\scene");
        invalid_reports.push(unsafe_target);

        let mut invalid_rva = found_report();
        invalid_rva["operation"]["callbackRva"] = json!(4096);
        invalid_reports.push(invalid_rva);

        let mut invalid_digest = found_report();
        invalid_digest["writerPath"][0]["codeSha256"] = json!("z".repeat(64));
        invalid_reports.push(invalid_digest);

        let mut invalid_schema = found_report();
        invalid_schema["operation"]["parameterSchema"] = json!("pointer-addresses");
        invalid_reports.push(invalid_schema);

        let mut null_field = found_report();
        null_field["writerPath"] = Value::Null;
        invalid_reports.push(null_field);

        invalid_reports.push(json!({
            "contractId": CONTRACT_ID,
            "resolutionStatus": "stale",
            "runtimeAbi": 4
        }));

        for invalid in invalid_reports {
            let result =
                sanitize_native_report(&encoded(&invalid)).unwrap_or_else(invalid_provider_report);
            assert_eq!(
                result,
                json!({"resolutionStatus":"provider-report-invalid"})
            );
            assert!(!result.to_string().contains("private"));
            assert!(!result.to_string().contains("0x12345678"));
        }
    }

    #[test]
    fn rejects_paths_beyond_the_native_bound_and_payloads_over_64_kib() {
        let mut overlong = found_report();
        overlong["writerCandidateRva"] = json!(263);
        overlong["writerPath"] = json!(
            (256..=263)
                .map(|rva| json!({"rva":rva,"codeSha256":"d".repeat(64)}))
                .collect::<Vec<_>>()
        );
        assert!(sanitize_native_report(&encoded(&overlong)).is_none());

        assert!(sanitize_native_report(&vec![b' '; MAX_METADATA_BYTES]).is_none());
    }
}
