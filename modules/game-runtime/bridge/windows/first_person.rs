use super::provider::provider;
use std::ffi::c_char;

const READY: u32 = 0;
const UNSUPPORTED_BUILD: u32 = 1;
const INVALID_ARGUMENT: u32 = 2;
const OWNER_CONFLICT: u32 = 3;

const MIN_CAMERA_DISTANCE: f64 = -0.5;
const MAX_CAMERA_DISTANCE: f64 = 0.5;
const HELD_ENTRY_UNAVAILABLE: &str = "native first-person provider lacks held-entry startup preparation; restart with a matching provider";

pub fn first_person_available() -> bool {
    let Some(provider) = provider() else {
        return false;
    };
    if !provider.first_person_held_entry_prepare {
        return false;
    }
    let (Some(available), Some(_configure), Some(_set_mode), Some(_release_owner)) = (
        provider.first_person_available,
        provider.first_person_configure,
        provider.first_person_set_mode,
        provider.first_person_release_owner,
    ) else {
        return false;
    };
    unsafe { available() }
}

pub fn first_person_detail() -> String {
    let Some(provider) = provider() else {
        return "native first-person provider detail is unavailable".into();
    };
    if !provider.first_person_held_entry_prepare {
        return HELD_ENTRY_UNAVAILABLE.into();
    }
    let Some(call) = provider.first_person_detail else {
        return "native first-person provider detail is unavailable".into();
    };

    let required = unsafe { call(std::ptr::null_mut(), 0) };
    if !(2..=4096).contains(&required) {
        return "native first-person provider returned an invalid detail length".into();
    }
    let mut bytes = vec![0_u8; required];
    let actual = unsafe { call(bytes.as_mut_ptr().cast::<c_char>(), bytes.len()) };
    if actual != required || bytes.last() != Some(&0) {
        return "native first-person provider detail changed while being copied".into();
    }
    bytes.pop();
    String::from_utf8(bytes)
        .unwrap_or_else(|_| "native first-person provider returned non-UTF-8 detail".into())
}

pub fn first_person_set_mode(
    owner_token: u64,
    enabled: bool,
    camera_distance: f64,
) -> Result<(), String> {
    if owner_token == 0
        || !camera_distance.is_finite()
        || !(MIN_CAMERA_DISTANCE..=MAX_CAMERA_DISTANCE).contains(&camera_distance)
    {
        return Err("invalid first-person owner token or camera distance".into());
    }
    dispatch_mode(
        enabled,
        || first_person_release_owner(owner_token),
        || enable_mode(owner_token, camera_distance),
    )
}

fn enable_mode(owner_token: u64, camera_distance: f64) -> Result<(), String> {
    let Some(provider) = provider() else {
        return Err("native first-person provider is unavailable".into());
    };
    if !provider.first_person_held_entry_prepare {
        return Err(HELD_ENTRY_UNAVAILABLE.into());
    }
    let (Some(configure), Some(set_mode), Some(release_owner)) = (
        provider.first_person_configure,
        provider.first_person_set_mode,
        provider.first_person_release_owner,
    ) else {
        return Err(first_person_detail());
    };
    if !provider
        .first_person_available
        .is_some_and(|available| unsafe { available() })
    {
        return Err(first_person_detail());
    }

    let configured = unsafe { configure(owner_token, camera_distance) };
    if configured != READY {
        let reason = status_reason(configured);
        return Err(release_after_failure(owner_token, release_owner, reason));
    }
    let mode = unsafe { set_mode(owner_token, true) };
    if mode != READY {
        let reason = status_reason(mode);
        return Err(release_after_failure(owner_token, release_owner, reason));
    }
    Ok(())
}

fn dispatch_mode(
    enabled: bool,
    release_owner: impl FnOnce() -> Result<(), String>,
    enable_mode: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if enabled {
        enable_mode()
    } else {
        release_owner()
    }
}

fn release_after_failure(
    owner_token: u64,
    release_owner: super::native_abi::FirstPersonReleaseOwner,
    reason: String,
) -> String {
    let cleanup = unsafe { release_owner(owner_token) };
    if cleanup == READY {
        reason
    } else {
        format!(
            "{reason}; owner cleanup also failed: {}",
            status_reason(cleanup)
        )
    }
}

/// Release is idempotent. Older ABI-12 providers cannot have acquired a
/// first-person owner, so absence of the optional export is a successful no-op.
pub fn first_person_release_owner(owner_token: u64) -> Result<(), String> {
    if owner_token == 0 {
        return Err("invalid first-person owner token".into());
    }
    let Some(call) = provider().and_then(|provider| provider.first_person_release_owner) else {
        return Ok(());
    };
    let status = unsafe { call(owner_token) };
    if status == READY {
        Ok(())
    } else {
        Err(status_reason(status))
    }
}

fn status_reason(status: u32) -> String {
    let message = match status {
        READY => "first-person control operation completed",
        UNSUPPORTED_BUILD => "first-person control is unsupported for this client build",
        INVALID_ARGUMENT => "native first-person control rejected an invalid argument",
        OWNER_CONFLICT => "native first-person owner token conflicts with another owner",
        _ => "native first-person control operation failed",
    };
    let detail = first_person_detail();
    if detail.is_empty() || detail == message {
        message.into()
    } else {
        format!("{message}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_owner_or_camera_distance_before_provider_access() {
        assert!(first_person_set_mode(0, true, 0.0).is_err());
        assert!(first_person_set_mode(1, true, f64::NAN).is_err());
        assert!(first_person_set_mode(1, true, f64::INFINITY).is_err());
        assert!(first_person_set_mode(1, true, -0.500_001).is_err());
        assert!(first_person_set_mode(1, true, 0.500_001).is_err());
        assert_eq!(
            first_person_release_owner(0).unwrap_err(),
            "invalid first-person owner token"
        );
    }

    #[test]
    fn disabling_releases_owner_without_requiring_current_availability() {
        let mut released_owner = None;
        let mut availability_was_checked = false;
        let result = dispatch_mode(
            false,
            || {
                released_owner = Some(42);
                Ok(())
            },
            || {
                availability_was_checked = true;
                Err("first-person is unavailable after admission changed".into())
            },
        );
        assert!(result.is_ok());
        assert_eq!(released_owner, Some(42));
        assert!(!availability_was_checked);
    }
}
