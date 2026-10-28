//! Any request to the API's routes, served in-process without TLS: the
//! bearer token, the path, the media type, `Content-Length`, `If-Match`,
//! the body limits and the inflation limit, then every handler. The server
//! never answers 500 or more, and a body over a limit is refused as too
//! large.
//!
//! The input is six choice bytes, four bytes of a declared length, then
//! three parts split at the first two zero bytes: the path text, a header
//! text, the body. In the path text and the body, `%T` stands for a spare
//! tenant's id, `%U` for a stored vCon's uuid and `%V` for its current
//! version, so the fuzzer can reach the routes that need them.
#![no_main]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use libfuzzer_sys::fuzz_target;
use tower::ServiceExt;
use vcon_server::access::{Scope, Scopes};
use vcon_server::api::{self, App, Limits};
use vcon_server::audit::Actor;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::fs::OsFs;
use vcon_server::ids::TenantId;
use vcon_server::retention::Retention;
use vcon_server::tenants::{Removal, Tenants};

const BODY_LIMIT: usize = 2048;
const INFLATED_LIMIT: u64 = 8192;
const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

const ROUTES: [(&str, &str); 25] = [
    ("POST", "/vcons"),
    ("GET", "/vcons/{}"),
    ("PUT", "/vcons/{}"),
    ("DELETE", "/vcons/{}"),
    ("GET", "/health"),
    ("GET", "/legal/vcons/{}"),
    ("GET", "/legal/vcons/{}/versions"),
    ("GET", "/legal/vcons/{}/versions/{}"),
    ("POST", "/legal/vcons/{}/restore"),
    ("PUT", "/legal/vcons/{}/hold"),
    ("DELETE", "/legal/vcons/{}/hold"),
    ("POST", "/admin/tenants"),
    ("GET", "/admin/tenants"),
    ("PATCH", "/admin/tenants/{}"),
    ("DELETE", "/admin/tenants/{}"),
    ("POST", "/admin/tenants/{}/keys"),
    ("GET", "/admin/tenants/{}/keys"),
    ("DELETE", "/admin/keys/{}"),
    ("GET", "/{}"),
    ("GET", "/vcons{}"),
    ("GET", "/audit{}"),
    ("GET", "/admin/audit{}"),
    ("GET", "/key"),
    ("GET", "/openapi.json"),
    ("GET", "/console/{}"),
];
const CREATE: usize = 0;
const TENANT_CREATE: usize = 11;
const TENANT_REMOVE: usize = 14;

const MEDIA_TYPES: [Option<&str>; 5] = [
    None,
    Some("application/vcon"),
    Some("application/vcon+gzip"),
    Some("application/json"),
    Some("text/plain"),
];

/// The server and what the inputs refer to.
struct World {
    runtime: tokio::runtime::Runtime,
    router: Router,
    tenants: Arc<Tenants>,
    /// Holds every scope a tenant key can.
    all: String,
    /// Holds write only.
    write: String,
    /// An operator key.
    admin: String,
    /// The tenant `%T` names, made again when a request removes it.
    spare: Mutex<TenantId>,
    /// The current version of the stored vCon, as `%V` gives it.
    version: Mutex<String>,
}

static DATA: OnceLock<PathBuf> = OnceLock::new();

extern "C" fn remove_data() {
    if let Some(dir) = DATA.get() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn world() -> &'static World {
    static WORLD: OnceLock<World> = OnceLock::new();
    WORLD.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("vcon-fuzz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).expect("data directory");
        DATA.set(dir.clone()).expect("set once");
        // SAFETY: registers a function with no arguments that only
        // removes the data directory; libFuzzer leaves through exit().
        unsafe { libc::atexit(remove_data) };
        let master = Key::generate().expect("a locked page");
        let tenants = Arc::new(
            Tenants::open(&dir, master, Arc::new(OsFs), Arc::new(SystemClock)).expect("open"),
        );
        let home = tenants.create(Actor::Cli, "home", None).expect("tenant");
        let spare = tenants.create(Actor::Cli, "spare", None).expect("tenant");
        let key = |tenant, scopes: &[Scope], label| {
            tenants
                .create_key(Actor::Cli, tenant, Scopes::of(scopes), label)
                .expect("key")
                .token
        };
        let all = key(
            Some(home),
            &[Scope::Write, Scope::Read, Scope::Delete, Scope::Legal],
            "all",
        );
        let write = key(Some(home), &[Scope::Write], "write");
        let admin = key(None, &[Scope::Admin], "admin");
        let app = App {
            tenants: tenants.clone(),
            limits: Limits {
                body: BODY_LIMIT,
                inflated: INFLATED_LIMIT,
                body_timeout: Duration::from_secs(5),
            },
            supported: BTreeSet::from([vcon_fuzz::SUPPORTED.to_owned()]),
            retention_default: Retention::Years(7),
            mcp: vcon_server::mcp::Limits::DEFAULT,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let world = World {
            runtime,
            router: api::router(Arc::new(app)),
            tenants,
            all,
            write,
            admin,
            spare: Mutex::new(spare),
            version: Mutex::new(String::new()),
        };
        let body =
            format!(r#"{{"uuid":"{UUID}","created_at":"2026-10-25T00:00:00Z","parties":[]}}"#);
        let request = Request::post("/vcons")
            .header("authorization", format!("Bearer {}", world.all))
            .header("content-type", "application/vcon")
            .body(Body::from(body))
            .expect("request");
        let answer = world
            .runtime
            .block_on(world.router.clone().oneshot(request))
            .expect("infallible");
        assert_eq!(
            answer.status(),
            StatusCode::CREATED,
            "the setup stores a valid vCon with a key that may write"
        );
        world.remember_version(&answer);
        world
    })
}

impl World {
    fn remember_version(&self, answer: &axum::response::Response) {
        if let Some(tag) = answer.headers().get("etag").and_then(|v| v.to_str().ok()) {
            *self.version.lock().expect("lock") = tag.trim_matches('"').to_owned();
        }
    }

    fn substitute(&self, text: &[u8]) -> Vec<u8> {
        let spare = self.spare.lock().expect("lock").to_string();
        let version = self.version.lock().expect("lock").clone();
        let mut out = Vec::with_capacity(text.len());
        let mut rest = text;
        while let Some((&b, tail)) = rest.split_first() {
            let with: Option<&str> = match (b, tail.first()) {
                (b'%', Some(b'T')) => Some(&spare),
                (b'%', Some(b'U')) => Some(UUID),
                (b'%', Some(b'V')) => Some(&version),
                _ => None,
            };
            match with {
                Some(text) => {
                    out.extend_from_slice(text.as_bytes());
                    rest = &tail[1..];
                }
                None => {
                    out.push(b);
                    rest = tail;
                }
            }
        }
        out
    }
}

/// The request an input describes, and whether its body or declared
/// length is over the body limit.
struct Described {
    route: usize,
    key: u8,
    media: Option<String>,
    known_media: bool,
    over: bool,
    body: Vec<u8>,
}

fuzz_target!(|data: &[u8]| {
    let Some((choices, rest)) = data.split_first_chunk::<10>() else {
        return;
    };
    let [route, key, media, length, chunk, if_match, n0, n1, n2, n3] = *choices;
    let world = world();
    let mut parts = rest.splitn(3, |&b| b == 0);
    let path_text = world.substitute(parts.next().unwrap_or_default());
    let header_text = parts.next().unwrap_or_default();
    let body = world.substitute(parts.next().unwrap_or_default());
    let Ok(path_text) = std::str::from_utf8(&path_text) else {
        return;
    };
    let route = usize::from(route) % ROUTES.len();
    let (method, template) = ROUTES[route];
    let uri = template.replace("{}", path_text);

    let mut request = Request::builder().method(method).uri(&uri);
    let authorization = match key % 5 {
        0 => None,
        1 => Some(format!("Bearer {}", world.all).into_bytes()),
        2 => Some(format!("Bearer {}", world.write).into_bytes()),
        3 => Some(format!("Bearer {}", world.admin).into_bytes()),
        _ => Some(header_text.to_vec()),
    };
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    let (media, known_media) = match usize::from(media) % (MEDIA_TYPES.len() + 1) {
        i if i < MEDIA_TYPES.len() => (MEDIA_TYPES[i].map(str::to_owned), true),
        _ => (
            std::str::from_utf8(header_text).ok().map(str::to_owned),
            false,
        ),
    };
    if let Some(media) = &media {
        request = request.header("content-type", media.as_str());
    }
    let declared = u64::from(u32::from_le_bytes([n0, n1, n2, n3]));
    let declared = match length % 4 {
        0 => None,
        1 => Some(body.len().to_string()),
        2 => Some(declared.to_string()),
        _ => std::str::from_utf8(header_text).ok().map(str::to_owned),
    };
    let over = body.len() > BODY_LIMIT
        || declared
            .as_deref()
            .and_then(|d| d.parse::<u64>().ok())
            .is_some_and(|d| d > BODY_LIMIT as u64);
    if let Some(declared) = &declared {
        request = request.header("content-length", declared.as_str());
    }
    match if_match % 4 {
        0 => {}
        1 => {
            let version = world.version.lock().expect("lock").clone();
            request = request.header("if-match", format!("\"{version}\""));
        }
        2 => request = request.header("if-match", "*"),
        _ => request = request.header("if-match", header_text),
    }
    // A body in one piece, or in chunks, as it may arrive.
    let request_body = if chunk == 0 || body.is_empty() {
        Body::from(body.clone())
    } else {
        let chunks: Vec<Result<Bytes, std::io::Error>> = body
            .chunks(usize::from(chunk))
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        Body::from_stream(futures_util::stream::iter(chunks))
    };
    let Ok(request) = request.body(request_body) else {
        return;
    };
    let answer = world
        .runtime
        .block_on(world.router.clone().oneshot(request))
        .expect("infallible");
    let status = answer.status();
    assert!(status.as_u16() < 500, "{method} {uri} answered {status}");
    if uri.contains(UUID) && status.is_success() {
        world.remember_version(&answer);
    }
    let described = Described {
        route,
        key,
        media,
        known_media,
        over,
        body,
    };
    check_limits(&described, status);
    tidy(world, &described, status, answer);
});

/// A create with a key that may write is refused as too large exactly
/// when its body or declared length is over the body limit, or its body
/// inflates past the inflation limit.
fn check_limits(request: &Described, status: StatusCode) {
    if request.route != CREATE || !matches!(request.key % 5, 1 | 2) {
        return;
    }
    if request.over {
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "over the body limit");
        return;
    }
    if !request.known_media || request.media.as_deref() == Some("text/plain") {
        return;
    }
    let limits = vcon::read::Limits::new(INFLATED_LIMIT);
    let supported = BTreeSet::from([vcon_fuzz::SUPPORTED.to_owned()]);
    let inflated_too_far =
        vcon::read::inspect(&request.body, request.media.as_deref(), &limits, &supported)
            == Err(vcon::read::Rejection::TooLarge);
    assert_eq!(
        status == StatusCode::PAYLOAD_TOO_LARGE,
        inflated_too_far,
        "413 must mean over a limit, and over a limit must mean 413 ({status})"
    );
}

/// Keeps the world the same size: a tenant a request made is removed, and
/// the spare tenant is made again if a request removed it.
fn tidy(world: &World, request: &Described, status: StatusCode, answer: axum::response::Response) {
    if request.route == TENANT_CREATE && status == StatusCode::CREATED {
        let body = world
            .runtime
            .block_on(axum::body::to_bytes(answer.into_body(), usize::MAX))
            .expect("in memory");
        let made: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        let id = made["id"]
            .as_str()
            .and_then(TenantId::parse)
            .expect("a tenant id");
        world
            .tenants
            .remove(Actor::Cli, id, Removal::ForceOverridingHolds)
            .expect("remove");
    }
    if request.route == TENANT_REMOVE && status == StatusCode::NO_CONTENT {
        let mut spare = world.spare.lock().expect("lock");
        if !world
            .tenants
            .list(Actor::Cli)
            .expect("list")
            .iter()
            .any(|t| t.id == *spare)
        {
            *spare = world
                .tenants
                .create(Actor::Cli, "spare", None)
                .expect("tenant");
        }
    }
}
