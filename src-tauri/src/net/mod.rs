//! The network gate (ROADMAP §0.1): the only way the app makes an outbound
//! request.
//!
//! Every request names its [`Service`]. Before anything is sent, the gate
//! reads the user's opt-in for that service from `service_optin`. If the
//! user hasn't opted in, the request is refused and no connection is
//! attempted, not even a DNS lookup. The app is offline by default.
//!
//! Requests to one service are spaced at least [`Service::min_interval`]
//! apart (MusicBrainz: 1 per second), across the whole app. The opt-in is
//! checked again after a request's wait, so opting out takes effect at once,
//! even for a request already waiting its turn.
//!
//! Every request carries a descriptive User-Agent, has a connect timeout and
//! an overall timeout, and must use HTTPS. Redirects are never followed: a
//! 3xx comes back to the caller, so nothing reaches a host the caller
//! didn't name. Errors name the host, never the full URL. There are no API secrets in the
//! binary: services that need a key get the user's own, passed in by the
//! caller as a header (ROADMAP §5.6).
//!
//! No other module may make HTTP calls or open sockets; a test scans the
//! source and the dependency tree for them (`tests/net_gate.rs`). The HTTP
//! client, `ureq`, stays private to this module.

mod limiter;
pub mod navigation;
mod optin;
mod service;
#[cfg(test)]
mod tests;

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::db::{DbError, ReadPool};
use limiter::RateLimiter;
pub use optin::{is_opted_in, set_opt_in};
pub use service::Service;

/// Sent with every request. MusicBrainz asks for the app's name, version
/// and a way to reach its author.
pub const USER_AGENT: &str = concat!(
    "tracklist-pro/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/hjzaradich/tracklist-pro )"
);

/// How long to wait for a connection to open.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The app's one rate limiter, shared by every [`Net`], so the limits hold
/// however many handles exist.
static LIMITER: LazyLock<Arc<RateLimiter>> = LazyLock::new(Arc::default);

/// A request to send through the gate.
#[derive(Debug, Clone)]
pub struct Request {
    service: Service,
    url: String,
    headers: Vec<(String, String)>,
    /// `Some` for a POST.
    body: Option<Vec<u8>>,
}

impl Request {
    /// A GET request to `service`.
    pub fn get(service: Service, url: impl Into<String>) -> Request {
        Request {
            service,
            url: url.into(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// A POST request to `service` with `body`.
    pub fn post(service: Service, url: impl Into<String>, body: Vec<u8>) -> Request {
        Request {
            body: Some(body),
            ..Request::get(service, url)
        }
    }

    /// Adds a header, e.g. the user's own API key for the service.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The service this request goes to.
    pub fn service(&self) -> Service {
        self.service
    }
}

/// A response. Error statuses (4xx, 5xx) and redirects (3xx) are responses
/// too, so callers can react to them, e.g. slow down on MusicBrainz's 503.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// The media type, without parameters, e.g. `application/json`.
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Why a request didn't get a response.
#[derive(Debug)]
pub enum NetError {
    /// The user hasn't opted in to this service. Nothing was sent.
    NotOptedIn(Service),
    /// The opt-in couldn't be read, so nothing was sent.
    OptInUnreadable(DbError),
    /// The response body was bigger than the service allows.
    TooLarge { service: Service, limit: u64 },
    /// The request failed: a bad or non-HTTPS URL, no connection, a
    /// timeout, a TLS error. Says only which host and what kind of failure:
    /// never the full URL, whose query may hold the user's key.
    Failed {
        host: Option<String>,
        reason: &'static str,
    },
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::NotOptedIn(s) => write!(f, "not opted in to {s}; nothing was sent"),
            NetError::OptInUnreadable(e) => {
                write!(f, "couldn't read the opt-in, so nothing was sent: {e}")
            }
            NetError::TooLarge { service, limit } => {
                write!(f, "{service} sent more than {limit} bytes")
            }
            NetError::Failed { host, reason } => match host {
                Some(host) => write!(f, "request to {host} failed: {reason}"),
                None => write!(f, "request failed: {reason}"),
            },
        }
    }
}

impl std::error::Error for NetError {}

/// A handle to the gate. Cheap to clone and safe to share between threads.
#[derive(Clone)]
pub struct Net {
    reads: ReadPool,
    agent: ureq::Agent,
    limiter: Arc<RateLimiter>,
    /// Replaces every service's overall timeout. Only tests set it.
    timeout_override: Option<Duration>,
}

impl Net {
    /// The gate, reading opt-ins through `reads`.
    pub fn new(reads: ReadPool) -> Net {
        Net {
            reads,
            agent: agent(true),
            limiter: Arc::clone(&LIMITER),
            timeout_override: None,
        }
    }

    /// Sends `request` if the user has opted in to its service, once its
    /// turn comes. Blocks until the response has been read.
    pub fn send(&self, request: Request) -> Result<Response, NetError> {
        let service = request.service;
        self.check_opt_in(service)?;
        self.limiter.wait(service, service.min_interval());
        // The user may have opted out while this request waited its turn.
        self.check_opt_in(service)?;
        // However long the opt-in read took, the next request waits a full
        // interval from now, when this one actually goes out.
        self.limiter.sending(service, service.min_interval());
        self.transmit(request)
    }

    fn check_opt_in(&self, service: Service) -> Result<(), NetError> {
        match is_opted_in(&self.reads, service) {
            Ok(true) => Ok(()),
            Ok(false) => Err(NetError::NotOptedIn(service)),
            Err(e) => Err(NetError::OptInUnreadable(e)),
        }
    }

    /// Sends a request that has passed the gate. The only place a
    /// connection is made.
    fn transmit(&self, request: Request) -> Result<Response, NetError> {
        let Request {
            service,
            url,
            headers,
            body,
        } = request;
        let timeout = Some(self.timeout_override.unwrap_or(service.timeout()));
        let failed = |e: ureq::Error| match e {
            ureq::Error::BodyExceedsLimit(limit) => NetError::TooLarge { service, limit },
            e => NetError::Failed {
                host: host_of(&url),
                reason: failure_reason(&e),
            },
        };
        let response = match body {
            None => {
                let mut builder = self.agent.get(&url);
                for (name, value) in &headers {
                    builder = builder.header(name, value);
                }
                builder.config().timeout_global(timeout).build().call()
            }
            Some(body) => {
                let mut builder = self.agent.post(&url);
                for (name, value) in &headers {
                    builder = builder.header(name, value);
                }
                builder
                    .config()
                    .timeout_global(timeout)
                    .build()
                    .send(&body[..])
            }
        }
        .map_err(failed)?;
        let status = response.status().as_u16();
        let body = response.into_body();
        let content_type = body.mime_type().map(str::to_owned);
        let body = body
            .into_with_config()
            .limit(service.max_body())
            .read_to_vec()
            .map_err(failed)?;
        Ok(Response {
            status,
            content_type,
            body,
        })
    }
}

/// The host a URL names, without its user info, port, path or query.
fn host_of(url: &str) -> Option<String> {
    let uri: ureq::http::Uri = url.parse().ok()?;
    uri.host().map(str::to_owned)
}

/// What kind of failure `e` is, in words that can't hold any part of the
/// request. ureq's own messages can include the full URL.
fn failure_reason(e: &ureq::Error) -> &'static str {
    use ureq::Error as E;
    match e {
        E::BadUri(_) => "the URL isn't valid",
        E::RequireHttpsOnly(_) => "the URL isn't HTTPS",
        E::HostNotFound => "the host wasn't found",
        E::ConnectionFailed | E::Io(_) => "the connection failed",
        E::Timeout(_) => "it timed out",
        E::Tls(_) | E::Pem(_) => "the secure connection failed",
        E::Protocol(_) | E::LargeResponseHeader(..) => "the reply wasn't valid HTTP",
        E::TooManyRedirects | E::RedirectFailed => "it was redirected",
        _ => "the request failed",
    }
}

/// The HTTP client's settings. `https_only` is always true outside tests.
fn agent(https_only: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(https_only)
        // Error statuses come back as responses; see [`Response`].
        .http_status_as_error(false)
        // Never follow a redirect: a 3xx comes back as a response. Following
        // one would send the caller's headers (e.g. a user's key) to
        // whatever host it names, and skip the gate and the rate limit.
        // Model downloads will follow redirects by hand, through the gate
        // (Phase 2).
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        // Never route through a proxy named in an environment variable: every
        // request goes straight to the service the user opted in to.
        .proxy(None)
        // Windows' own TLS and certificate store (SChannel), so no root list
        // is bundled and security fixes come with Windows updates.
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}
