//! Explicit owner capability for the core's opt-in, loopback-only HTTP surface.
//! The caller owns issuance and delivery. No credential is generated or persisted here.

use std::fmt;

use anyhow::{Result, bail};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct AdminGate {
    digest: [u8; 32],
    authority: String,
    base_path: String,
}

impl AdminGate {
    /// Accept an owner-supplied capability without retaining its plaintext.
    /// The owner must supply an unpredictable value and deliver it out of band.
    /// Only the Authorization header is accepted; URLs and cookies are not credentials.
    /// Public static UI assets contain no runtime data; API and SSE remain authenticated.
    pub fn new(capability: &str) -> Result<Self> {
        if !valid_bearer_token(capability) {
            bail!("admin capability must be a nonempty Bearer token");
        }
        Ok(Self {
            digest: Sha256::digest(capability.as_bytes()).into(),
            authority: String::new(),
            base_path: String::new(),
        })
    }

    pub(super) fn bind(mut self, address: std::net::SocketAddr, base_path: &str) -> Self {
        self.authority = address.to_string();
        self.base_path = base_path.trim_end_matches('/').to_owned();
        self
    }

    fn safe_origin(&self, headers: &HeaderMap) -> bool {
        let single = |name: header::HeaderName| {
            let mut values = headers.get_all(name).iter();
            let value = values.next().and_then(|v| v.to_str().ok());
            if values.next().is_some() { None } else { value }
        };
        if single(header::HOST) != Some(self.authority.as_str()) {
            return false;
        }
        if headers.contains_key(header::ORIGIN)
            && single(header::ORIGIN) != Some(format!("http://{}", self.authority).as_str())
        {
            return false;
        }
        !headers
            .get("sec-fetch-site")
            .is_some_and(|v| v != "same-origin" && v != "none")
    }

    fn public_shell(&self, request: &Request) -> bool {
        if request.method() != axum::http::Method::GET {
            return false;
        }
        let path = request.uri().path();
        if path == self.base_path || path == format!("{}/", self.base_path) {
            return true;
        }
        let prefix = format!("{}/assets/", self.base_path);
        path.strip_prefix(&prefix).is_some_and(|asset| {
            matches!(asset, "memory-chan-hero.png" | "memory-chan-sidebar.png")
                || super::assets::frontend_asset(asset).is_some()
        })
    }

    fn authorizes(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all(header::AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return false;
        };
        // Multiple credentials are ambiguous, even if one happens to be valid.
        if values.next().is_some() {
            return false;
        }
        let Ok(value) = value.to_str() else {
            return false;
        };
        let Some((scheme, capability)) = value.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("Bearer") || !valid_bearer_token(capability) {
            return false;
        }
        let supplied: [u8; 32] = Sha256::digest(capability.as_bytes()).into();
        bool::from(self.digest.ct_eq(&supplied))
    }
}

impl fmt::Debug for AdminGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdminGate { capability: [REDACTED] }")
    }
}

fn valid_bearer_token(value: &str) -> bool {
    let unpadded = value.trim_end_matches('=');
    !unpadded.is_empty()
        && unpadded.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        })
}

pub(super) async fn require_admin(
    State(gate): State<AdminGate>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut response = if !gate.safe_origin(request.headers()) {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"local same-origin access required"})),
        )
            .into_response()
    } else if gate.authorizes(request.headers())
        || (!request.headers().contains_key(header::AUTHORIZATION) && gate.public_shell(&request))
    {
        // Downstream handlers have no reason to inspect or accidentally log the credential.
        request.headers_mut().remove(header::AUTHORIZATION);
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Bearer realm=\"memory-ledger-admin\"",
            )],
            Json(json!({ "error": "admin authorization required" })),
        )
            .into_response()
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Authorization"));
    for (name, value) in [
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-resource-policy", "same-origin"),
        ("x-frame-options", "DENY"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
