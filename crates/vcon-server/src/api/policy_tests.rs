//! The policy of every route for every role, generated from the one
//! table of roles to scopes (`crate::roles`): a role reaches a route if
//! and only if its scopes hold the scope the route needs.
//!
//! `POLICY` states what each route needs. It is held to the description
//! (`openapi.json`), which the unit tests hold to the router, so a route
//! added without its line here fails. Each role is then given a key
//! holding exactly the role's scopes, the form a membership's scopes take
//! until sessions exist, and every route is called with it: the answer is
//! 403 where the table says the role does not reach the route, and is not
//! 403 where it says it does.

use std::collections::BTreeSet;

use reqwest::Method;

use super::tests::pipeline::{Server, server};
use crate::access::{Scope, Scopes};
use crate::audit::Actor;
use crate::roles::{Role, Roles};

/// What a route asks of its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// A key holding this scope.
    Scope(Scope),
    /// Any valid key, of a tenant or the operator.
    AnyKey,
    /// Nothing: the route takes no key.
    NoKey,
}

const POLICY: &[(&str, &str, Needs)] = &[
    ("POST", "/vcons", Needs::Scope(Scope::Write)),
    ("GET", "/vcons", Needs::Scope(Scope::Read)),
    ("GET", "/vcons/{uuid}", Needs::Scope(Scope::Read)),
    ("PUT", "/vcons/{uuid}", Needs::Scope(Scope::Write)),
    ("DELETE", "/vcons/{uuid}", Needs::Scope(Scope::Delete)),
    ("GET", "/health", Needs::AnyKey),
    ("GET", "/openapi.json", Needs::AnyKey),
    ("POST", "/mcp", Needs::AnyKey),
    ("GET", "/key", Needs::AnyKey),
    ("GET", "/console", Needs::NoKey),
    ("GET", "/console/", Needs::NoKey),
    ("GET", "/console/console.js", Needs::NoKey),
    ("GET", "/console/console.css", Needs::NoKey),
    ("GET", "/console/format.js", Needs::NoKey),
    ("GET", "/console/player.html", Needs::NoKey),
    ("GET", "/console/player.js", Needs::NoKey),
    ("POST", "/console/csp-report", Needs::NoKey),
    ("GET", "/console/{other}", Needs::NoKey),
    ("POST", "/console/{other}", Needs::NoKey),
    ("PUT", "/console/{other}", Needs::NoKey),
    ("PATCH", "/console/{other}", Needs::NoKey),
    ("DELETE", "/console/{other}", Needs::NoKey),
    ("GET", "/audit", Needs::Scope(Scope::Audit)),
    ("GET", "/admin/audit", Needs::Scope(Scope::Admin)),
    ("GET", "/legal/vcons/{uuid}", Needs::Scope(Scope::Legal)),
    (
        "GET",
        "/legal/vcons/{uuid}/versions",
        Needs::Scope(Scope::Legal),
    ),
    (
        "GET",
        "/legal/vcons/{uuid}/versions/{version}",
        Needs::Scope(Scope::Legal),
    ),
    (
        "POST",
        "/legal/vcons/{uuid}/restore",
        Needs::Scope(Scope::Legal),
    ),
    (
        "PUT",
        "/legal/vcons/{uuid}/hold",
        Needs::Scope(Scope::Legal),
    ),
    (
        "DELETE",
        "/legal/vcons/{uuid}/hold",
        Needs::Scope(Scope::Legal),
    ),
    ("GET", "/admin/tenants", Needs::Scope(Scope::Admin)),
    ("POST", "/admin/tenants", Needs::Scope(Scope::Admin)),
    ("PATCH", "/admin/tenants/{id}", Needs::Scope(Scope::Admin)),
    ("DELETE", "/admin/tenants/{id}", Needs::Scope(Scope::Admin)),
    (
        "PUT",
        "/admin/tenants/{id}/metadata",
        Needs::Scope(Scope::Admin),
    ),
    (
        "GET",
        "/admin/tenants/{id}/keys",
        Needs::Scope(Scope::Admin),
    ),
    (
        "POST",
        "/admin/tenants/{id}/keys",
        Needs::Scope(Scope::Admin),
    ),
    ("DELETE", "/admin/keys/{id}", Needs::Scope(Scope::Admin)),
];

fn policy() -> Vec<(&'static str, &'static str, Needs)> {
    POLICY.to_vec()
}

/// Whether a caller holding `scopes` reaches a route that needs `needs`.
fn reaches(scopes: Scopes, needs: Needs) -> bool {
    match needs {
        Needs::Scope(scope) => scopes.contains(scope),
        Needs::AnyKey | Needs::NoKey => true,
    }
}

/// A path of the route, with an id no row has in each place.
fn path(template: &str) -> String {
    template
        .replace("{uuid}", "0195a1f0-7c3e-7d4a-8b21-3f5e6a7b8c9d")
        .replace("{version}", "0123456789abcdef0123456789abcdef")
        .replace("/admin/keys/{id}", "/admin/keys/0123456789abcdef")
        .replace("{id}", "00000000000000000000000000000000")
        .replace("{other}", "nothing")
}

/// A key holding exactly `scopes`: an operator key for admin, and a key
/// of the server's tenant for any other set.
fn key(s: &Server, scopes: Scopes) -> String {
    let tenant = (!scopes.contains(Scope::Admin)).then_some(s.tenant);
    s.tenants
        .create_key(Actor::Cli, tenant, scopes, "policy")
        .expect("a key")
        .token
}

async fn status(s: &Server, method: &str, template: &str, token: Option<&str>) -> u16 {
    let method = Method::from_bytes(method.as_bytes()).expect("a method");
    let mut request = s
        .client_not_following()
        .request(method, s.url(&path(template)));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.expect("an answer").status().as_u16()
}

/// Calls every route with a key holding `scopes` and holds each answer to
/// the table.
async fn held_to_the_table(scopes: Scopes, who: &str) {
    let s = server().await;
    let token = key(&s, scopes);
    for (method, template, needs) in policy() {
        let answer = status(&s, method, template, Some(&token)).await;
        assert_ne!(answer, 401, "{who}: {method} {template} took the key");
        assert_eq!(
            answer != 403,
            reaches(scopes, needs),
            "{who}: {method} {template} needs {needs:?} and answered {answer}"
        );
    }
}

// What must work.

#[test]
fn the_table_names_every_route_the_description_lists() {
    let description: serde_json::Value =
        serde_json::from_str(super::OPENAPI).expect("the description");
    let mut listed = BTreeSet::new();
    for (route, methods) in description["paths"].as_object().expect("paths") {
        for method in methods.as_object().expect("methods").keys() {
            if method != "parameters" {
                listed.insert((method.to_uppercase(), route.clone()));
            }
        }
    }
    let table: BTreeSet<(String, String)> = policy()
        .into_iter()
        .map(|(method, route, _)| (method.to_owned(), route.to_owned()))
        .collect();
    assert_eq!(
        table.len(),
        policy().len(),
        "no route is in the table twice"
    );
    assert_eq!(
        table, listed,
        "the table and openapi.json name the same routes"
    );
}

#[tokio::test]
async fn a_reader_reaches_the_routes_that_need_read() {
    held_to_the_table(Role::Reader.scopes(), "reader").await;
}

#[tokio::test]
async fn a_producer_reaches_the_routes_that_need_write() {
    held_to_the_table(Role::Producer.scopes(), "producer").await;
}

#[tokio::test]
async fn records_reaches_the_route_that_needs_delete() {
    held_to_the_table(Role::Records.scopes(), "records").await;
}

#[tokio::test]
async fn legal_reaches_the_legal_routes() {
    held_to_the_table(Role::Legal.scopes(), "legal").await;
}

#[tokio::test]
async fn an_auditor_reaches_the_tenants_audit_log() {
    held_to_the_table(Role::Auditor.scopes(), "auditor").await;
}

#[tokio::test]
async fn an_operator_reaches_the_admin_routes() {
    held_to_the_table(Role::Operator.scopes(), "operator").await;
}

#[tokio::test]
async fn two_roles_reach_the_routes_of_either() {
    for (one, other) in [
        (Role::Reader, Role::Auditor),
        (Role::Producer, Role::Records),
        (Role::Legal, Role::TenantAdmin),
    ] {
        let roles = Roles::of(&[one, other]);
        held_to_the_table(roles.scopes(), &roles.to_string()).await;
    }
}

#[tokio::test]
async fn every_tenant_role_together_reaches_every_route_but_the_admin_ones() {
    held_to_the_table(Roles::TENANT.scopes(), "every tenant role").await;
}

#[test]
fn each_scope_but_tenant_admin_is_needed_by_a_route() {
    for scope in Scope::ALL {
        let routes = policy()
            .into_iter()
            .filter(|(_, _, needs)| *needs == Needs::Scope(scope))
            .count();
        // No route needs tenant_admin yet: the tenant routes that do are
        // not in this build.
        assert_eq!(routes > 0, scope != Scope::TenantAdmin, "{scope:?}");
    }
}

#[test]
fn each_route_that_needs_a_scope_is_reached_by_one_role() {
    for (method, route, needs) in policy() {
        let roles: Vec<Role> = Role::ALL
            .into_iter()
            .filter(|role| reaches(role.scopes(), needs))
            .collect();
        match needs {
            Needs::Scope(_) => assert_eq!(roles.len(), 1, "{method} {route}: {roles:?}"),
            Needs::AnyKey | Needs::NoKey => {
                assert_eq!(roles.len(), Role::ALL.len(), "{method} {route}: {roles:?}");
            }
        }
    }
}

#[test]
fn the_roles_reach_the_routes_the_plan_gives_them() {
    // The count of routes each role's scope opens, read from the table:
    // a change of a route's scope or a role's scope moves a number here.
    let opened = |role: Role| {
        policy()
            .into_iter()
            .filter(|(_, _, needs)| {
                matches!(needs, Needs::Scope(scope) if role.scopes().contains(*scope))
            })
            .count()
    };
    assert_eq!(
        Role::ALL.map(opened),
        [2, 2, 1, 6, 1, 0, 9],
        "reader, producer, records, legal, auditor, tenant-admin, operator"
    );
}

#[tokio::test]
async fn the_keyless_routes_answer_with_no_key() {
    let s = server().await;
    for (method, template, needs) in policy() {
        if needs == Needs::NoKey {
            let answer = status(&s, method, template, None).await;
            assert!(
                answer != 401 && answer != 403,
                "{method} {template} answered {answer}"
            );
        }
    }
}

// What must be refused.

#[tokio::test]
async fn a_tenant_admin_reaches_no_route_that_needs_a_scope() {
    let scopes = Role::TenantAdmin.scopes();
    assert!(
        policy()
            .into_iter()
            .all(|(_, _, needs)| !matches!(needs, Needs::Scope(_)) || !reaches(scopes, needs)),
        "the table gives tenant_admin no route"
    );
    held_to_the_table(scopes, "tenant-admin").await;
}

#[tokio::test]
async fn every_route_that_takes_a_key_answers_401_with_none() {
    let s = server().await;
    for (method, template, needs) in policy() {
        if needs != Needs::NoKey {
            assert_eq!(
                status(&s, method, template, None).await,
                401,
                "{method} {template}"
            );
        }
    }
}

#[tokio::test]
async fn every_route_that_takes_a_key_answers_401_to_a_token_no_key_has() {
    let s = server().await;
    let unknown = format!("vcs_0123456789abcdef_{}", "A".repeat(43));
    for (method, template, needs) in policy() {
        if needs != Needs::NoKey {
            assert_eq!(
                status(&s, method, template, Some(&unknown)).await,
                401,
                "{method} {template}"
            );
        }
    }
}

#[tokio::test]
async fn a_revoked_key_reaches_no_route_whatever_its_role() {
    let s = server().await;
    let made = s
        .tenants
        .create_key(Actor::Cli, Some(s.tenant), Roles::TENANT.scopes(), "gone")
        .expect("a key");
    s.tenants.revoke_key(Actor::Cli, made.id).expect("revoked");
    for (method, template, needs) in policy() {
        if needs != Needs::NoKey {
            assert_eq!(
                status(&s, method, template, Some(&made.token)).await,
                401,
                "{method} {template}"
            );
        }
    }
}

#[tokio::test]
async fn no_tenant_role_or_set_of_them_reaches_an_admin_route() {
    let s = server().await;
    let token = key(&s, Roles::TENANT.scopes());
    for (method, template, needs) in policy() {
        if needs == Needs::Scope(Scope::Admin) {
            assert_eq!(
                status(&s, method, template, Some(&token)).await,
                403,
                "{method} {template}"
            );
        }
    }
}

#[tokio::test]
async fn an_operator_reaches_no_route_of_a_tenant() {
    let s = server().await;
    let token = key(&s, Role::Operator.scopes());
    for (method, template, needs) in policy() {
        if matches!(needs, Needs::Scope(scope) if scope != Scope::Admin) {
            assert_eq!(
                status(&s, method, template, Some(&token)).await,
                403,
                "{method} {template}"
            );
        }
    }
}

#[tokio::test]
async fn a_role_the_cap_leaves_out_opens_no_route() {
    // A membership of reader and records in a tenant capped at reader has
    // the scopes of reader alone.
    let held = Roles::of(&[Role::Reader, Role::Records]);
    let counted = held.within(Some(Roles::of(&[Role::Reader])));
    assert_eq!(
        counted.scopes(),
        Role::Reader.scopes(),
        "the scopes that count"
    );
    let s = server().await;
    let token = key(&s, counted.scopes());
    assert_eq!(
        status(&s, "DELETE", "/vcons/{uuid}", Some(&token)).await,
        403,
        "delete is refused"
    );
    assert_ne!(
        status(&s, "GET", "/vcons", Some(&token)).await,
        403,
        "read is not"
    );
}

#[tokio::test]
async fn a_membership_wholly_outside_the_cap_has_no_scope_a_key_could_hold() {
    let counted = Roles::of(&[Role::Legal]).within(Some(Roles::of(&[Role::Reader])));
    assert!(counted.scopes().is_empty(), "no scope");
    let s = server().await;
    assert_eq!(
        s.tenants
            .create_key(Actor::Cli, Some(s.tenant), counted.scopes(), "none")
            .err(),
        Some(crate::tenants::TenantError::BadScopes),
        "a key needs a scope"
    );
}

// Limits and odd inputs.

#[test]
fn a_path_is_made_for_every_route_with_no_place_left_open() {
    for (method, template, _) in policy() {
        let made = path(template);
        assert!(
            !made.contains('{') && !made.contains('}'),
            "{method} {template} became {made}"
        );
    }
}

#[test]
fn the_table_has_a_line_for_each_method_of_a_route_apart() {
    let hold: Vec<&str> = policy()
        .into_iter()
        .filter(|(_, route, _)| *route == "/legal/vcons/{uuid}/hold")
        .map(|(method, _, _)| method)
        .collect();
    assert_eq!(hold, ["PUT", "DELETE"], "the two methods of one route");
}

#[test]
fn reaching_is_read_from_the_scopes_alone() {
    let none = Scopes::default();
    assert!(reaches(none, Needs::AnyKey), "any key, whatever it holds");
    assert!(reaches(none, Needs::NoKey), "no key at all");
    for scope in Scope::ALL {
        assert!(
            !reaches(none, Needs::Scope(scope)),
            "{scope:?} with no scope"
        );
        assert!(
            reaches(Scopes::of(&[scope]), Needs::Scope(scope)),
            "{scope:?} with itself"
        );
    }
}

#[test]
fn every_pair_of_tenant_roles_reaches_the_union_of_their_routes() {
    let tenant: Vec<Role> = Role::ALL
        .into_iter()
        .filter(|role| *role != Role::Operator)
        .collect();
    for one in &tenant {
        for other in &tenant {
            let both = Roles::of(&[*one, *other]).scopes();
            for (method, route, needs) in policy() {
                assert_eq!(
                    reaches(both, needs),
                    reaches(one.scopes(), needs) || reaches(other.scopes(), needs),
                    "{one:?} and {other:?}: {method} {route}"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_key_of_one_role_holds_on_the_tenant_listener_of_a_split_server() {
    use super::tests::pipeline::server_on;
    let s = server_on(crate::access::Plane::Tenant).await;
    let token = key(&s, Role::Auditor.scopes());
    assert_ne!(
        status(&s, "GET", "/audit", Some(&token)).await,
        403,
        "the auditor's route"
    );
    assert_eq!(
        status(&s, "GET", "/vcons", Some(&token)).await,
        403,
        "a route of another role"
    );
}
