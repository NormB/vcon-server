//! The console: a web page served on the TLS listener at `/console/`. It
//! is a client of the HTTP API and nothing more. Its files are built
//! into the binary and need no key: they are the same for every caller,
//! hold no data and change nothing. Every request the page makes is an API
//! request that carries the key the user signed in with.

use axum::Router;
use axum::extract::Request;
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};

/// One console file: its name under `/console/` (empty for the page),
/// its bytes as built into the binary, and its media type.
pub type File = (&'static str, &'static [u8], &'static str);

/// Every file the console is made of. The router serves exactly these, and
/// a test holds this list, the `console` directory and the package's
/// `include` list to one another.
pub const FILES: [File; 4] = [
    (
        "index.html",
        include_bytes!("../console/index.html"),
        "text/html; charset=utf-8",
    ),
    (
        "console.js",
        include_bytes!("../console/console.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "format.js",
        include_bytes!("../console/format.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "console.css",
        include_bytes!("../console/console.css"),
        "text/css; charset=utf-8",
    ),
];

/// The file the bare `/console/` path answers with.
const PAGE: &str = "index.html";

/// The path a console file is served at.
#[must_use]
pub fn path_of(name: &str) -> String {
    if name == PAGE {
        "/console/".to_owned()
    } else {
        format!("/console/{name}")
    }
}

/// The headers on every console answer: no inline code, nothing from
/// another origin, no framing, no sniffing, no referrer, no caching.
pub const HEADERS: [(&str, &str); 7] = [
    (
        "content-security-policy",
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
         connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; \
         require-trusted-types-for 'script'; trusted-types 'none'",
    ),
    ("x-frame-options", "DENY"),
    ("x-content-type-options", "nosniff"),
    ("referrer-policy", "no-referrer"),
    ("cache-control", "no-store"),
    ("cross-origin-opener-policy", "same-origin"),
    ("cross-origin-resource-policy", "same-origin"),
];

/// The console's routes, merged into the API's router: its files,
/// `/console` sent on to `/console/`, and 404 for any other path under
/// it, each answer with the headers.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    let mut router = Router::new().route(
        "/console",
        get(|| async { (StatusCode::PERMANENT_REDIRECT, [(LOCATION, "/console/")]) }),
    );
    for (name, body, media_type) in FILES {
        router = router.route(&path_of(name), get(move || file(body, media_type)));
    }
    router
        .route("/console/{*other}", any(|| async { StatusCode::NOT_FOUND }))
        // On the console's own routes only, so the API's answers and its
        // fallback are left as they are.
        .route_layer(middleware::from_fn(with_headers))
}

async fn file(body: &'static [u8], media_type: &'static str) -> Response {
    ([(CONTENT_TYPE, media_type)], body).into_response()
}

/// Puts every header in `HEADERS` on a console answer.
async fn with_headers(request: Request, next: Next) -> Response {
    let mut answer = next.run(request).await;
    for (name, value) in HEADERS {
        answer.headers_mut().insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    answer
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn source(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("console")
            .join(name);
        std::fs::read_to_string(path).expect("the source file")
    }

    /// Every script of the console, by name.
    fn scripts() -> Vec<(&'static str, String)> {
        super::FILES
            .into_iter()
            .filter(|(name, _, _)| name.ends_with(".js"))
            .map(|(name, _, _)| (name, source(name)))
            .collect()
    }

    /// The directory, the served list and the package's include list name
    /// the same files, so a file added to one and not the others fails
    /// here and not as a 404 in a browser or a broken published crate.
    #[test]
    fn the_directory_the_served_files_and_the_package_agree() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("console");
        let mut on_disk: Vec<String> = std::fs::read_dir(directory)
            .expect("the console directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .into_string()
                    .expect("a UTF-8 name")
            })
            .collect();
        on_disk.sort();
        let mut served: Vec<String> = super::FILES
            .into_iter()
            .map(|(name, _, _)| name.to_owned())
            .collect();
        served.sort();
        assert_eq!(served, on_disk, "served files against the directory");
        let manifest =
            std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
                .expect("the manifest");
        for name in &on_disk {
            assert!(
                manifest.contains(&format!("\"/console/{name}\",")),
                "the package includes {name}"
            );
        }
    }

    /// Each file is served with the media type its extension needs: a
    /// module script under another type is refused by the browser.
    #[test]
    fn each_file_has_the_media_type_of_its_extension() {
        for (name, body, media_type) in super::FILES {
            let expected = match name.rsplit('.').next() {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                other => panic!("{name}: no media type for {other:?}"),
            };
            assert_eq!(media_type, expected, "{name}");
            assert!(!body.is_empty(), "{name} is not empty");
            assert!(std::str::from_utf8(body).is_ok(), "{name} is UTF-8");
        }
    }

    #[test]
    fn the_page_is_served_at_the_directory_path_and_the_rest_by_name() {
        assert_eq!(super::path_of("index.html"), "/console/", "index.html");
        assert_eq!(
            super::path_of("console.js"),
            "/console/console.js",
            "console.js"
        );
        assert_eq!(
            super::path_of("console.css"),
            "/console/console.css",
            "console.css"
        );
        assert_eq!(
            super::path_of("format.js"),
            "/console/format.js",
            "format.js"
        );
    }

    /// The headers hold the rules the design sets, so a weaker policy
    /// fails here, not only in a browser: the CSP allows nothing by
    /// default, each source is the console's own origin or none, and no
    /// script may run but the console's file; the page is never framed,
    /// cached, sniffed or sent a referrer.
    #[test]
    fn the_headers_are_the_designed_ones() {
        let headers: std::collections::BTreeMap<&str, &str> = super::HEADERS.into_iter().collect();
        assert_eq!(headers.len(), super::HEADERS.len(), "each header once");
        let csp: std::collections::BTreeMap<&str, Vec<&str>> = headers["content-security-policy"]
            .split(';')
            .map(|d| {
                let mut words = d.split_whitespace();
                (words.next().expect("a directive"), words.collect())
            })
            .collect();
        for (directive, sources) in [
            ("default-src", ["'none'"]),
            ("script-src", ["'self'"]),
            ("frame-ancestors", ["'none'"]),
            ("base-uri", ["'none'"]),
            ("form-action", ["'none'"]),
            ("require-trusted-types-for", ["'script'"]),
            ("trusted-types", ["'none'"]),
        ] {
            assert_eq!(csp.get(directive), Some(&sources.to_vec()), "{directive}");
        }
        for (directive, sources) in &csp {
            assert!(
                sources
                    .iter()
                    .all(|s| ["'none'", "'self'", "'script'"].contains(s)),
                "{directive} allows only the console's own origin: {sources:?}"
            );
        }
        for (name, value) in [
            ("x-frame-options", "DENY"),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("cache-control", "no-store"),
            ("cross-origin-opener-policy", "same-origin"),
            ("cross-origin-resource-policy", "same-origin"),
        ] {
            assert_eq!(headers.get(name), Some(&value), "{name}");
        }
    }

    /// The page loads its one script and one stylesheet and holds no
    /// inline script, style or event handler, which the CSP would refuse.
    #[test]
    fn the_page_holds_no_inline_code() {
        let page = source("index.html");
        assert!(
            page.contains(r#"<script type="module" src="console.js"></script>"#),
            "the page loads its script"
        );
        assert!(
            page.contains(r#"<link rel="stylesheet" href="console.css">"#),
            "the page loads its styles"
        );
        let lower = page.to_lowercase();
        assert_eq!(lower.matches("<script").count(), 1, "one script element");
        assert!(!lower.contains("<style"), "no style element");
        assert!(!lower.contains(" style="), "no style attribute");
        assert!(!has_event_handler(&page), "no event-handler attribute");
    }

    /// Whether markup holds an attribute such as `onclick=`: whitespace,
    /// `on`, a name of letters, then `=` after any spaces.
    fn has_event_handler(html: &str) -> bool {
        let lower = html.to_lowercase();
        lower
            .match_indices("on")
            .filter(|(at, _)| lower[..*at].ends_with(char::is_whitespace))
            .any(|(at, _)| {
                let rest = &lower[at + 2..];
                let name = rest.chars().take_while(char::is_ascii_lowercase).count();
                name > 0 && rest[name..].trim_start().starts_with('=')
            })
    }

    #[test]
    fn an_event_handler_is_found_wherever_it_is() {
        for found in [
            "<b onclick=\"x\">",
            "<b\nonload = x>",
            "<img src=a ONERROR=x>",
        ] {
            assert!(has_event_handler(found), "{found}");
        }
        for clean in [
            "<b class=on>",
            "<b>on=</b>",
            "<a href=\"x\">done</a>",
            "<b on=x>",
        ] {
            assert!(!has_event_handler(clean), "{clean}");
        }
    }

    /// No script reaches the page through markup, runs code from strings
    /// or keeps anything in browser storage; values are set as text.
    #[test]
    fn no_script_uses_an_html_sink_or_storage() {
        for (name, script) in scripts() {
            for refused in [
                "innerHTML",
                "outerHTML",
                "insertAdjacentHTML",
                "document.write",
                "eval(",
                "Function(",
                "setTimeout(\"",
                "setInterval(\"",
                "localStorage",
                "sessionStorage",
                "indexedDB",
                "document.cookie",
                "caches.",
            ] {
                assert!(!script.contains(refused), "{name} uses {refused}");
            }
        }
        assert!(
            source("console.js").contains("textContent"),
            "values are set as text"
        );
    }

    /// Every request goes through one call of `fetch`, which never lets
    /// the browser keep the answer, send a cookie or follow a redirect.
    #[test]
    fn every_request_goes_through_one_fetch() {
        let calls: usize = scripts()
            .iter()
            .map(|(_, script)| script.matches("fetch(").count())
            .sum();
        assert_eq!(calls, 1, "one fetch call in all the scripts");
        let script = source("console.js");
        for option in [
            r#"cache: "no-store""#,
            r#"credentials: "omit""#,
            r#"redirect: "error""#,
        ] {
            assert!(script.contains(option), "fetch is made with {option}");
        }
    }
}
