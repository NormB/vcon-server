//! The player's page over HTTPS: the one console answer whose policy lets
//! a recording play, and proof that no other answer's policy changed.

use super::tests::pipeline::server;
use crate::console;

/// The policy every console answer has carried since the console shipped,
/// written out so that a change to any answer's policy fails here.
const STRICT: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
     connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; \
     require-trusted-types-for 'script'; trusted-types 'none'; \
     report-uri /console/csp-report";

/// The policy of the player's page, written out.
const PLAYER: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
     connect-src 'none'; media-src blob:; base-uri 'none'; form-action 'none'; \
     frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'; \
     report-uri /console/csp-report";

#[tokio::test]
async fn the_players_page_is_served_with_the_policy_that_plays_a_blob() {
    let s = server().await;
    let answer = s
        .client()
        .get(s.url("/console/player.html"))
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status(), 200, "the status");
    assert_eq!(
        answer.headers()["content-type"],
        "text/html; charset=utf-8",
        "the media type"
    );
    assert_eq!(
        answer.headers()["content-security-policy"],
        PLAYER,
        "the policy"
    );
    for (name, value) in &console::HEADERS[1..] {
        assert_eq!(answer.headers()[*name], *value, "{name}");
    }
}

/// A refusal at the player's path is not the player's page: it carries
/// the console's policy, as every refusal does.
#[tokio::test]
async fn a_request_the_players_path_refuses_has_the_strict_policy() {
    let s = server().await;
    for request in [
        s.client().post(s.url("/console/player.html")),
        s.client().put(s.url("/console/player.html")),
        s.client().delete(s.url("/console/player.html")),
    ] {
        let answer = request.send().await.expect("an answer");
        assert_eq!(answer.status(), 405, "{answer:?}");
        assert_eq!(
            answer.headers()["content-security-policy"],
            STRICT,
            "{answer:?}"
        );
    }
}

/// The policy of every console answer but the player's page is the one
/// it always was, byte for byte: each file, the redirect, a path that is
/// not a file, a method a path does not take, and paths that are nearly
/// the player's.
#[tokio::test]
async fn every_console_answer_but_the_players_page_keeps_the_strict_policy() {
    let s = server().await;
    let client = s.client_not_following();
    let mut asked = 0;
    for (file, _, _) in console::FILES {
        let path = console::path_of(file);
        if path == console::PLAYER_PATH {
            continue;
        }
        let answer = client.get(s.url(&path)).send().await.expect("an answer");
        assert_eq!(answer.status(), 200, "{path}");
        assert_eq!(
            answer.headers()["content-security-policy"],
            STRICT,
            "{path}"
        );
        asked += 1;
    }
    assert_eq!(
        asked,
        console::FILES.len() - 1,
        "every file but the player's page"
    );
    for (path, status) in [
        ("/console", 308),
        ("/console/other.js", 404),
        ("/console/player.html/", 404),
        ("/console/player.html/x", 404),
        ("/console/PLAYER.HTML", 404),
        ("/console/player.htm", 404),
        ("/console/x/player.html", 404),
        ("/console/player.html%2F", 404),
    ] {
        let answer = client.get(s.url(path)).send().await.expect("an answer");
        assert_eq!(answer.status(), status, "{path}");
        assert_eq!(
            answer.headers()["content-security-policy"],
            STRICT,
            "{path}"
        );
    }
    let refused = client
        .post(s.url("/console/"))
        .send()
        .await
        .expect("an answer");
    assert_eq!(refused.status(), 405, "a method the page does not take");
    assert_eq!(
        refused.headers()["content-security-policy"],
        STRICT,
        "a refusal"
    );
    assert_eq!(
        console::HEADERS[0],
        ("content-security-policy", STRICT),
        "the shared headers"
    );
}
