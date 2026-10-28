//! The `http_request` target's server and checks, here so unit tests can
//! drive the same world through a sequence of inputs.
//!
//! An input is any request to the API's routes, served in-process without
//! TLS: the bearer token, the path, the media type, `Content-Length`,
//! `If-Match`, the body limits and the inflation limit, then every handler.
//! The server never answers 500 or more, and a body over a limit is refused
//! as too large.
//!
//! The input is ten choice bytes (route, key, media type, declared length,
//! chunk size, `If-Match`, and four bytes of a declared length), then three
//! parts split at the first two zero bytes: the path text, a header text,
//! the body. In the path text and the body, `%T` stands for a spare
//! tenant's id, `%U` for a stored vCon's uuid and `%V` for its current
//! version, so the fuzzer can reach the routes that need them.
//!
//! One world serves every input of a run, so a request may change it: the
//! fuzzer can remove a tenant or revoke a key once it has learned its id
//! from a comparison. After each request the world is put back as it was:
//! the home tenant, its stored vCon and the three keys are made again when
//! a request removed or revoked them, and the spare tenant when a request
//! removed it.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use tower::ServiceExt;
use vcon_server::access::{Scope, Scopes};
use vcon_server::api;
use vcon_server::api::{App, Limits};
use vcon_server::audit::Actor;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId};
use vcon_server::retention::Retention;
use vcon_server::tenants::{Removal, Tenants};

/// The body limit the server is built with.
pub const BODY_LIMIT: usize = 2048;
/// The inflation limit the server is built with.
pub const INFLATED_LIMIT: u64 = 8192;
/// The uuid of the vCon the home tenant holds.
pub const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

/// Each route an input's first byte picks, as method and path template.
pub const ROUTES: [(&str, &str); 25] = [
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
/// `POST /vcons`.
pub const CREATE: usize = 0;
/// `POST /admin/tenants`.
pub const TENANT_CREATE: usize = 11;
/// `DELETE /admin/tenants/{}`.
pub const TENANT_REMOVE: usize = 14;
/// `DELETE /admin/keys/{}`.
pub const KEY_REVOKE: usize = 17;

const MEDIA_TYPES: [Option<&str>; 5] = [
    None,
    Some("application/vcon"),
    Some("application/vcon+gzip"),
    Some("application/json"),
    Some("text/plain"),
];

/// The world's three keys: their tokens, and their ids.
struct Keys {
    /// Holds every scope a tenant key can.
    all: String,
    /// Holds write only.
    write: String,
    /// An operator key.
    admin: String,
    ids: [KeyId; 3],
}

/// The server and what the inputs refer to.
pub struct World {
    runtime: tokio::runtime::Runtime,
    router: Router,
    tenants: Arc<Tenants>,
    /// The tenant the keys `all` and `write` belong to, which holds the
    /// vCon `%U` names.
    home: Mutex<TenantId>,
    keys: Mutex<Keys>,
    /// The tenant `%T` names, made again when a request removes it.
    spare: Mutex<TenantId>,
    /// The current version of the stored vCon, as `%V` gives it.
    version: Mutex<String>,
}

impl World {
    /// A server on a new data directory `dir`, with a home tenant holding
    /// one vCon, its keys, an operator key and a spare tenant.
    pub fn new(dir: &Path) -> World {
        let master = Key::generate().expect("a locked page");
        let tenants = Arc::new(
            Tenants::open(dir, master, Arc::new(OsFs), Arc::new(SystemClock)).expect("open"),
        );
        let spare = tenants.create(Actor::Cli, "spare", None).expect("tenant");
        let app = App {
            tenants: tenants.clone(),
            limits: Limits {
                body: BODY_LIMIT,
                inflated: INFLATED_LIMIT,
                body_timeout: Duration::from_secs(5),
            },
            supported: BTreeSet::from([crate::SUPPORTED.to_owned()]),
            retention_default: Retention::Years(7),
            mcp: vcon_server::mcp::Limits::DEFAULT,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let admin = tenants
            .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "admin")
            .expect("key");
        let world = World {
            runtime,
            router: api::router(Arc::new(app)),
            tenants,
            home: Mutex::new(TenantId([0; 16])),
            keys: Mutex::new(Keys {
                all: String::new(),
                write: String::new(),
                admin: admin.token,
                ids: [KeyId([0; 8]), KeyId([0; 8]), admin.id],
            }),
            spare: Mutex::new(spare),
            version: Mutex::new(String::new()),
        };
        world.make_home();
        world
    }

    /// Makes the home tenant, its two keys and its vCon.
    fn make_home(&self) {
        let home = self
            .tenants
            .create(Actor::Cli, "home", None)
            .expect("tenant");
        *self.home.lock().expect("lock") = home;
        self.make_keys(home);
        self.store_vcon();
    }

    /// Makes the two keys of the home tenant `home`.
    fn make_keys(&self, home: TenantId) {
        let key = |scopes: &[Scope], label| {
            self.tenants
                .create_key(Actor::Cli, Some(home), Scopes::of(scopes), label)
                .expect("key")
        };
        let all = key(
            &[Scope::Write, Scope::Read, Scope::Delete, Scope::Legal],
            "all",
        );
        let write = key(&[Scope::Write], "write");
        let mut keys = self.keys.lock().expect("lock");
        keys.ids[0] = all.id;
        keys.ids[1] = write.id;
        keys.all = all.token;
        keys.write = write.token;
    }

    /// Stores the vCon `%U` names in the home tenant.
    fn store_vcon(&self) {
        let body =
            format!(r#"{{"uuid":"{UUID}","created_at":"2026-10-25T00:00:00Z","parties":[]}}"#);
        let all = self.keys.lock().expect("lock").all.clone();
        let request = Request::post("/vcons")
            .header("authorization", format!("Bearer {all}"))
            .header("content-type", "application/vcon")
            .body(Body::from(body))
            .expect("request");
        let answer = self.send(request);
        assert_eq!(
            answer.status(),
            StatusCode::CREATED,
            "the setup stores a valid vCon with a key that may write"
        );
        self.remember_version(&answer);
    }

    /// Checks that `token`, just revoked, is refused: 401, before any
    /// limit, as for a create over the body limit.
    fn refused(&self, token: &str) {
        let request = Request::post("/vcons")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/vcon")
            .header("content-length", (BODY_LIMIT + 1).to_string())
            .body(Body::from(vec![b' '; BODY_LIMIT + 1]))
            .expect("request");
        assert_eq!(
            self.send(request).status(),
            StatusCode::UNAUTHORIZED,
            "a revoked key is refused before its body is read"
        );
    }

    fn send(&self, request: Request<Body>) -> Response {
        self.runtime
            .block_on(self.router.clone().oneshot(request))
            .expect("infallible")
    }

    fn remember_version(&self, answer: &Response) {
        if let Some(tag) = answer.headers().get("etag").and_then(|v| v.to_str().ok()) {
            *self.version.lock().expect("lock") = tag.trim_matches('"').to_owned();
        }
    }

    /// The token of the key with every tenant scope.
    pub fn all_token(&self) -> String {
        self.keys.lock().expect("lock").all.clone()
    }

    /// The id of the home tenant.
    pub fn home(&self) -> TenantId {
        *self.home.lock().expect("lock")
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

    /// Sends the request `data` describes and checks the answer; panics on
    /// a broken property. Inputs too short to describe a request, and
    /// paths that are not UTF-8 or not a URI, are skipped.
    pub fn run(&self, data: &[u8]) {
        let Some((choices, rest)) = data.split_first_chunk::<10>() else {
            return;
        };
        let [route, key, media, length, chunk, if_match, n0, n1, n2, n3] = *choices;
        let mut parts = rest.splitn(3, |&b| b == 0);
        let path_text = self.substitute(parts.next().unwrap_or_default());
        let header_text = parts.next().unwrap_or_default();
        let body = self.substitute(parts.next().unwrap_or_default());
        let Ok(path_text) = std::str::from_utf8(&path_text) else {
            return;
        };
        let route = usize::from(route) % ROUTES.len();
        let (method, template) = ROUTES[route];
        let uri = template.replace("{}", path_text);

        let mut request = Request::builder().method(method).uri(&uri);
        let authorization = {
            let keys = self.keys.lock().expect("lock");
            match key % 5 {
                0 => None,
                1 => Some(format!("Bearer {}", keys.all).into_bytes()),
                2 => Some(format!("Bearer {}", keys.write).into_bytes()),
                3 => Some(format!("Bearer {}", keys.admin).into_bytes()),
                _ => Some(header_text.to_vec()),
            }
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
                let version = self.version.lock().expect("lock").clone();
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
        let answer = self.send(request);
        let status = answer.status();
        assert!(
            status.as_u16() < 500,
            "the server never fails: {method} {uri} answered {status}"
        );
        if uri.contains(UUID) && status.is_success() {
            self.remember_version(&answer);
        }
        let described = Described {
            route,
            path: path_text.to_owned(),
            key,
            media,
            known_media,
            over,
            body,
        };
        check_limits(&described, status);
        self.tidy(&described, status, answer);
    }

    /// Keeps the world the same: a tenant a request made is removed, the
    /// spare tenant is made again if a request removed it, and the home
    /// tenant, its vCon and the keys are made again if a request removed or
    /// revoked them, after checking that a revoked key is refused.
    fn tidy(&self, request: &Described, status: StatusCode, answer: Response) {
        if request.route == KEY_REVOKE && status == StatusCode::NO_CONTENT {
            let revoked = KeyId::parse(&request.path);
            let ids = self.keys.lock().expect("lock").ids;
            if revoked == Some(ids[2]) {
                let admin = self.keys.lock().expect("lock").admin.clone();
                self.refused(&admin);
                let admin = self
                    .tenants
                    .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "admin")
                    .expect("key");
                let mut keys = self.keys.lock().expect("lock");
                keys.ids[2] = admin.id;
                keys.admin = admin.token;
            } else if revoked.is_some_and(|id| ids[..2].contains(&id)) {
                let (all, write) = {
                    let keys = self.keys.lock().expect("lock");
                    (keys.all.clone(), keys.write.clone())
                };
                self.refused(if revoked == Some(ids[0]) {
                    &all
                } else {
                    &write
                });
                self.make_keys(self.home());
            }
        }
        if request.route == TENANT_CREATE && status == StatusCode::CREATED {
            let body = self
                .runtime
                .block_on(axum::body::to_bytes(answer.into_body(), usize::MAX))
                .expect("in memory");
            let made: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
            let id = made["id"]
                .as_str()
                .and_then(TenantId::parse)
                .expect("a tenant id");
            self.tenants
                .remove(Actor::Cli, id, Removal::ForceOverridingHolds)
                .expect("remove");
        }
        if request.route == TENANT_REMOVE && status == StatusCode::NO_CONTENT {
            let listed = self.tenants.list(Actor::Cli).expect("list");
            {
                let mut spare = self.spare.lock().expect("lock");
                if !listed.iter().any(|t| t.id == *spare) {
                    *spare = self
                        .tenants
                        .create(Actor::Cli, "spare", None)
                        .expect("tenant");
                }
            }
            if !listed.iter().any(|t| t.id == self.home()) {
                // A removal revokes the tenant's keys.
                self.refused(&self.all_token());
                self.make_home();
            }
        }
    }
}

/// The request an input describes, and whether its body or declared
/// length is over the body limit.
struct Described {
    route: usize,
    /// The path text, after `%T`, `%U` and `%V` are put in.
    path: String,
    key: u8,
    media: Option<String>,
    known_media: bool,
    over: bool,
    body: Vec<u8>,
}

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
    let supported = BTreeSet::from([crate::SUPPORTED.to_owned()]);
    let inflated_too_far =
        vcon::read::inspect(&request.body, request.media.as_deref(), &limits, &supported)
            == Err(vcon::read::Rejection::TooLarge);
    assert_eq!(
        status == StatusCode::PAYLOAD_TOO_LARGE,
        inflated_too_far,
        "413 must mean over a limit, and over a limit must mean 413 ({status})"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data directory of its own, removed when dropped.
    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir =
                std::env::temp_dir().join(format!("vcon-fuzz-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir(&dir).expect("data directory");
            Dir(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An input: the ten choice bytes, then the path text, the header text
    /// and the body.
    fn input(choices: [u8; 10], path: &str, header: &[u8], body: &[u8]) -> Vec<u8> {
        let mut data = choices.to_vec();
        data.extend_from_slice(path.as_bytes());
        data.push(0);
        data.extend_from_slice(header);
        data.push(0);
        data.extend_from_slice(body);
        data
    }

    /// The input CI's run 40678565535 crashed on: a create with the key
    /// `all`, a gzip media type and a declared length of the body's own,
    /// 2246 bytes, over the 2048-byte limit.
    const CRASH: &[u8] = include_bytes!("../seeds/http_request/create-after-the-key-is-revoked");

    /// The key id in a token, `vcs_<key id>_<secret>`.
    fn key_id(token: &str) -> String {
        token.split('_').nth(1).expect("a key id").to_owned()
    }

    #[test]
    fn the_crash_input_on_its_own_is_refused_as_too_large() {
        let dir = Dir::new("crash");
        let world = World::new(&dir.0);
        world.run(CRASH);
    }

    #[test]
    fn a_revoked_key_of_the_world_is_made_again() {
        let dir = Dir::new("revoked");
        let world = World::new(&dir.0);
        let revoked = world.all_token();
        // DELETE /admin/keys/<the key all> with the operator key.
        world.run(&input(
            [KEY_REVOKE as u8, 3, 0, 0, 0, 0, 0, 0, 0, 0],
            &key_id(&revoked),
            b"",
            b"",
        ));
        // Before the world was put back, this was CI's crash: 401, since
        // the key is revoked, where 413 was expected.
        world.run(CRASH);
        assert_ne!(
            world.all_token(),
            revoked,
            "the world holds a new key with every tenant scope"
        );
    }

    #[test]
    fn a_removed_home_tenant_is_made_again_with_its_keys_and_vcon() {
        let dir = Dir::new("home");
        let world = World::new(&dir.0);
        let home = world.home();
        // DELETE /admin/tenants/<home>?force=true with the operator key.
        world.run(&input(
            [TENANT_REMOVE as u8, 3, 0, 0, 0, 0, 0, 0, 0, 0],
            &format!("{home}?force=true"),
            b"",
            b"",
        ));
        world.run(CRASH);
        assert_ne!(world.home(), home, "the world holds a new home tenant");
        // GET /vcons/%U with the key all: the home tenant holds the vCon.
        let read = Request::get(format!("/vcons/{UUID}"))
            .header("authorization", format!("Bearer {}", world.all_token()))
            .body(Body::empty())
            .expect("request");
        assert_eq!(
            world.send(read).status(),
            StatusCode::OK,
            "the new home tenant holds the vCon %U names"
        );
    }
}
