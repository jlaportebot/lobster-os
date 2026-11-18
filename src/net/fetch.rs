//! Status-aware page fetch.
//!
//! The lobster-net device runs the host's curl and exposes the real HTTP
//! status next to the body, so a 404 or 500 page can be shown instead of
//! being reported as a network failure. The decision logic that interprets
//! the device registers is pure and lives here so it can be host-tested.

use alloc::vec::Vec;

/// Largest body the device hands over (its buffer minus the 16 bytes it keeps back).
pub const DEVICE_BODY_CAP: usize = 524_288 - 16;

/// Smallest HTTP status code curl can report.
const MIN_HTTP_STATUS: u32 = 100;
/// Largest HTTP status code curl can report.
const MAX_HTTP_STATUS: u32 = 599;

/// The device buffer holds 16 bytes more than the body cap.
const DEVICE_CAP_SLACK: usize = 16;

/// A completed HTTP exchange.
#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    /// HTTP status code (200, 404, ...).
    pub status: u16,
    /// Response body, at most [`DEVICE_BODY_CAP`] bytes.
    pub body: Vec<u8>,
    /// True when the body hit the device's size cap and was cut short.
    pub truncated: bool,
}

impl Response {
    /// True for 2xx statuses.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Why a fetch produced no response at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// No network device or stack is available.
    NoNetwork,
    /// The device reported a transport failure (DNS, refused, TLS, ...).
    Failed,
    /// The device did not finish within the polling budget.
    TimedOut,
    /// The URL cannot be sent to the device.
    InvalidUrl,
}

impl FetchError {
    /// Human-readable reason for an error page.
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            FetchError::NoNetwork => "No network device is available",
            FetchError::Failed => "The server could not be reached",
            FetchError::TimedOut => "The request timed out",
            FetchError::InvalidUrl => "The address is not valid",
        }
    }
}

/// What the device registers say about a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// Still running.
    Waiting,
    /// Finished: status code, body length and whether the body was capped.
    Ready {
        /// HTTP status code.
        status: u16,
        /// Body length in bytes.
        len: usize,
        /// Body hit the cap.
        truncated: bool,
    },
    /// Finished without a usable response.
    Failed(FetchError),
}

/// Raw device state sampled from the MMIO registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Request in flight (idle or busy).
    Pending,
    /// Request finished.
    Done,
    /// Request failed on the host side.
    Error,
}

/// Interpret the device registers once the request may have finished.
#[must_use]
pub fn classify(state: DeviceState, code: u32, len: u32) -> Completion {
    match state {
        DeviceState::Pending => Completion::Waiting,
        DeviceState::Error => Completion::Failed(FetchError::Failed),
        DeviceState::Done => classify_done(code, len),
    }
}

fn classify_done(code: u32, len: u32) -> Completion {
    if !(MIN_HTTP_STATUS..=MAX_HTTP_STATUS).contains(&code) {
        return Completion::Failed(FetchError::Failed);
    }
    let len = len as usize;
    if len > DEVICE_BODY_CAP + DEVICE_CAP_SLACK {
        return Completion::Failed(FetchError::Failed);
    }
    Completion::Ready {
        status: code as u16,
        len,
        truncated: len >= DEVICE_BODY_CAP,
    }
}

/// Short reason phrase for the status line.
#[must_use]
pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        410 => "Gone",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn pending_device_keeps_waiting() {
        assert_eq!(classify(DeviceState::Pending, 0, 0), Completion::Waiting);
    }

    #[test]
    fn ok_response_is_ready_with_length() {
        assert_eq!(
            classify(DeviceState::Done, 200, 1234),
            Completion::Ready {
                status: 200,
                len: 1234,
                truncated: false
            }
        );
    }

    #[test]
    fn error_status_keeps_its_body() {
        assert_eq!(
            classify(DeviceState::Done, 404, 77),
            Completion::Ready {
                status: 404,
                len: 77,
                truncated: false
            }
        );
        assert_eq!(
            classify(DeviceState::Done, 500, 0),
            Completion::Ready {
                status: 500,
                len: 0,
                truncated: false
            }
        );
    }

    #[test]
    fn empty_200_is_still_a_response() {
        assert_eq!(
            classify(DeviceState::Done, 200, 0),
            Completion::Ready {
                status: 200,
                len: 0,
                truncated: false
            }
        );
    }

    #[test]
    fn code_zero_means_transport_failure() {
        assert_eq!(
            classify(DeviceState::Done, 0, 0),
            Completion::Failed(FetchError::Failed)
        );
    }

    #[test]
    fn out_of_range_code_is_a_failure() {
        assert_eq!(
            classify(DeviceState::Done, 99, 1),
            Completion::Failed(FetchError::Failed)
        );
        assert_eq!(
            classify(DeviceState::Done, 600, 1),
            Completion::Failed(FetchError::Failed)
        );
    }

    #[test]
    fn device_error_is_a_failure() {
        assert_eq!(
            classify(DeviceState::Error, 0, 0),
            Completion::Failed(FetchError::Failed)
        );
    }

    #[test]
    fn body_at_cap_is_flagged_truncated() {
        let len = DEVICE_BODY_CAP as u32;
        assert_eq!(
            classify(DeviceState::Done, 200, len),
            Completion::Ready {
                status: 200,
                len: DEVICE_BODY_CAP,
                truncated: true
            }
        );
        assert_eq!(
            classify(DeviceState::Done, 200, len - 1),
            Completion::Ready {
                status: 200,
                len: DEVICE_BODY_CAP - 1,
                truncated: false
            }
        );
    }

    #[test]
    fn length_beyond_device_buffer_is_rejected() {
        let len = (DEVICE_BODY_CAP + 17) as u32;
        assert_eq!(
            classify(DeviceState::Done, 200, len),
            Completion::Failed(FetchError::Failed)
        );
    }

    #[test]
    fn success_covers_only_2xx() {
        let mk = |status| Response {
            status,
            body: Vec::new(),
            truncated: false,
        };
        assert!(mk(200).is_success());
        assert!(mk(204).is_success());
        assert!(!mk(301).is_success());
        assert!(!mk(404).is_success());
    }

    #[test]
    fn reason_phrases_cover_common_codes() {
        assert_eq!(reason_phrase(200), "OK");
        assert_eq!(reason_phrase(404), "Not Found");
        assert_eq!(reason_phrase(500), "Internal Server Error");
        assert_eq!(reason_phrase(418), "");
    }

    #[test]
    fn error_messages_are_distinct() {
        assert_ne!(FetchError::Failed.message(), FetchError::TimedOut.message());
    }
}
