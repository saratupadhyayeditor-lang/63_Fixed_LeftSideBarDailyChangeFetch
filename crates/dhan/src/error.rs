//! Error types for the DhanHQ client.
//!
//! Dhan returns errors as `{ "errorType", "errorCode", "errorMessage" }` with
//! codes in the `DH-9xx` range (see Annexure - Trading API Error). Transport and
//! WebSocket failures are surfaced separately so callers can distinguish "the
//! broker rejected this" from "the network broke".

use serde::Deserialize;

/// Error returned by every fallible DhanHQ operation.
#[derive(Debug, thiserror::Error)]
pub enum DhanError {
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("websocket error: {0}")]
    WebSocket(String),

    #[error("Dhan API error {code} ({error_type}): {message}")]
    Api {
        code: String,
        error_type: String,
        message: String,
    },

    #[error("invalid Dhan response: {0}")]
    Invalid(String),

    #[error("client is not connected to Dhan")]
    NotConnected,

    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, DhanError>;

/// Raw error envelope exactly as Dhan documents it.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DhanErrorBody {
    #[serde(default, alias = "errorType")]
    pub error_type: String,
    #[serde(default, alias = "errorCode")]
    pub error_code: String,
    #[serde(default, alias = "errorMessage")]
    pub error_message: String,
}

impl DhanError {
    /// True when Dhan asked us to slow down. Matches the documented `DH-904`
    /// code, but Dhan's data APIs also answer a burst with a bare HTTP 429 whose
    /// body is `{"data":{"805":"Too many requests..."}}` (no `errorCode`), so the
    /// status / message / error-type are all checked too. Missing these made the
    /// adaptive backoff never engage and the account stayed throttled.
    pub fn is_rate_limit(&self) -> bool {
        match self {
            DhanError::Api {
                code,
                error_type,
                message,
            } => {
                code == "DH-904"
                    || code.starts_with("429")
                    || code == "805"
                    || error_type.to_ascii_lowercase().contains("too many")
                    || {
                        let m = message.to_ascii_lowercase();
                        m.contains("too many request")
                            || m.contains("too many")
                            || m.contains("\"805\"")
                    }
            }
            _ => false,
        }
    }

    /// True when the access token / client id was rejected (`DH-901`).
    pub fn is_auth(&self) -> bool {
        matches!(self, DhanError::Api { code, .. } if code == "DH-901")
    }

    /// True when a holdings call simply found no delivery holdings. A clean
    /// account is answered with `DH-1111 (HOLDING_ERROR): No holdings available`
    /// instead of an empty list, which is not a real failure and must not be
    /// surfaced as an error in the Condition Log.
    pub fn is_empty_holdings(&self) -> bool {
        match self {
            DhanError::Api {
                code,
                error_type,
                message,
            } => {
                code == "DH-1111"
                    || error_type.eq_ignore_ascii_case("HOLDING_ERROR")
                    || message.to_ascii_lowercase().contains("no holdings")
            }
            _ => false,
        }
    }

    /// Build from an HTTP status plus the decoded JSON body. Falls back to a
    /// generic message when the body is not a Dhan error envelope.
    pub fn from_body(status: reqwest::StatusCode, body: &str) -> Self {
        let parsed: Option<DhanErrorBody> = serde_json::from_str(body).ok();
        if let Some(b) = parsed {
            if !b.error_code.is_empty() || !b.error_message.is_empty() {
                return DhanError::Api {
                    code: b.error_code,
                    error_type: b.error_type,
                    message: b.error_message,
                };
            }
        }
        DhanError::Api {
            code: status.as_u16().to_string(),
            error_type: status.canonical_reason().unwrap_or("HTTP").to_string(),
            message: body.trim().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: &str, error_type: &str, message: &str) -> DhanError {
        DhanError::Api {
            code: code.into(),
            error_type: error_type.into(),
            message: message.into(),
        }
    }

    #[test]
    fn empty_holdings_is_not_an_error() {
        assert!(api(
            "DH-1111",
            "HOLDING_ERROR",
            "No holdings available"
        )
        .is_empty_holdings());
        // Any one of the three signals is enough on its own.
        assert!(api("DH-1111", "", "").is_empty_holdings());
        assert!(api("", "HOLDING_ERROR", "").is_empty_holdings());
        assert!(api("", "", "No holdings available").is_empty_holdings());
        // Real failures stay failures.
        assert!(!api("DH-901", "AUTH_ERROR", "Invalid token").is_empty_holdings());
        assert!(!DhanError::NotConnected.is_empty_holdings());
    }

    #[test]
    fn detects_every_rate_limit_shape() {
        // Documented code.
        assert!(api("DH-904", "RATE_LIMIT", "slow down").is_rate_limit());
        // Bare HTTP 429 with Dhan's 805 payload (what the data APIs actually
        // return on a burst) must count as a rate limit.
        assert!(api(
            "429",
            "Too Many Requests",
            r#"{"data":{"805":"Too many requests. Further requests may result in the user being blocked."},"status":"failed"}"#
        )
        .is_rate_limit());
        assert!(api("805", "", "").is_rate_limit());
        assert!(api("", "TOO MANY", "").is_rate_limit());
        // Non-rate-limit failures stay false.
        assert!(!api("DH-901", "AUTH_ERROR", "Invalid token").is_rate_limit());
        assert!(!DhanError::NotConnected.is_rate_limit());
    }
}
