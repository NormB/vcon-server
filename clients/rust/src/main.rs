//! Works through the HTTP API with reqwest's blocking client: stores a
//! vCon, finds it in the list, reads it back, stores and reads back a
//! gzipped vCon (made with flate2), pages through both with a cursor,
//! replaces the first under If-Match, places and releases a legal hold,
//! finds its own create in the audit log, lists the tenants with an
//! operator key, reads the vCon over MCP, and deletes both.
//!
//! Set `VCON_URL` (`https://vcon.example.com`) and four keys: `VCON_KEY`
//! (the write, read and delete scopes), `VCON_LEGAL_KEY` (legal),
//! `VCON_AUDIT_KEY` (audit) and `VCON_OPERATOR_KEY` (an operator key). For
//! a certificate your system does not trust, set `VCON_CA` (its PEM file).

use std::io::{Read, Write};
use std::process::ExitCode;

use flate2::Compression;
use flate2::write::GzEncoder;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

type Failure = Box<dyn std::error::Error>;

struct Api {
    url: String,
    key: String,
    http: Client,
}

impl Api {
    /// Sends one request with `key` and the given headers. An answer of 400
    /// or more is a problem document, returned as an error with its type.
    fn send(
        &self,
        key: &str,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<Response, Failure> {
        let method = reqwest::Method::from_bytes(method.as_bytes())?;
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.url))
            .bearer_auth(key);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        let answer = request.send()?;
        if answer.status().as_u16() >= 400 {
            let status = answer.status().as_u16();
            let problem: Value = serde_json::from_slice(&answer.bytes()?)?;
            let kind = problem["type"].as_str().unwrap_or("no problem type");
            return Err(format!("{method} {path}: {status} {kind}").into());
        }
        Ok(answer)
    }

    /// Sends one request with `VCON_KEY`.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<Response, Failure> {
        self.send(&self.key, method, path, body, headers)
    }

    /// Sends a GET with `key` and parses the JSON answer.
    fn json(&self, key: &str, path: &str) -> Result<Value, Failure> {
        Ok(serde_json::from_slice(
            &self.send(key, "GET", path, None, &[])?.bytes()?,
        )?)
    }

    /// Sends one request that must be refused; returns the refusal's
    /// "METHOD PATH: status type" text.
    fn refused(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        headers: &[(&str, &str)],
    ) -> Result<String, Failure> {
        match self.call(method, path, body, headers) {
            Ok(_) => Err(format!("{method} {path} was not refused").into()),
            Err(refusal) => Ok(refusal.to_string()),
        }
    }

    /// Posts one JSON-RPC message to /mcp; returns the answer's result, or
    /// null for a notification.
    fn mcp(&self, message: Value) -> Result<Value, Failure> {
        let body = message.to_string();
        let headers = [
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ];
        let answer = self
            .call("POST", "/mcp", Some(body.as_bytes()), &headers)?
            .bytes()?;
        if answer.is_empty() {
            return Ok(Value::Null);
        }
        let rpc: Value = serde_json::from_slice(&answer)?;
        if !rpc["error"].is_null() {
            return Err(format!("MCP answered {}", rpc["error"]).into());
        }
        Ok(rpc["result"].clone())
    }
}

fn expect(holds: bool, what: &str) -> Result<(), Failure> {
    if holds { Ok(()) } else { Err(what.into()) }
}

/// A random (version 4) uuid in RFC 9562 text form.
fn new_uuid() -> Result<String, Failure> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// A vCon's JSON text; serde_json writes an object's keys sorted.
fn encode(uuid: &str, subject: &str) -> Vec<u8> {
    json!({
        "created_at": "2026-10-25T12:00:00Z",
        "parties": [],
        "subject": subject,
        "uuid": uuid,
    })
    .to_string()
    .into_bytes()
}

/// The ETag of an answer, or an empty string.
fn etag(answer: &Response) -> String {
    answer
        .headers()
        .get("ETag")
        .and_then(|tag| tag.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[expect(
    clippy::too_many_lines,
    reason = "one example, read from top to bottom, step by step"
)]
fn run() -> Result<(), Failure> {
    let mut http = Client::builder();
    if let Ok(path) = std::env::var("VCON_CA") {
        http = http.add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(path)?)?);
    }
    let api = Api {
        url: std::env::var("VCON_URL")?,
        key: std::env::var("VCON_KEY")?,
        http: http.build()?,
    };
    let legal_key = std::env::var("VCON_LEGAL_KEY")?;
    let audit_key = std::env::var("VCON_AUDIT_KEY")?;
    let operator_key = std::env::var("VCON_OPERATOR_KEY")?;
    let uuid = new_uuid()?;
    let vcon = encode(&uuid, "client example");
    let as_vcon = [("Content-Type", "application/vcon")];

    // Store: 201, with the new version in ETag.
    let stored = api.call("POST", "/vcons", Some(&vcon), &as_vcon)?;
    expect(
        stored.status() == StatusCode::CREATED,
        "a store did not answer 201",
    )?;
    let first_tag = etag(&stored);
    println!("stored {uuid}");

    // List: the newest vCon comes first.
    let page = api.json(&api.key, "/vcons?limit=10")?;
    let stored_at = page["vcons"]
        .as_array()
        .and_then(|vcons| vcons.iter().find(|v| v["uuid"] == uuid.as_str()))
        .and_then(|v| v["stored_at"].as_str())
        .ok_or("the stored vCon is not in the list")?
        .to_owned();
    println!("listed {uuid}");

    // Read: the body comes back exactly as stored.
    let body = api
        .call("GET", &format!("/vcons/{uuid}"), None, &[])?
        .bytes()?;
    expect(
        body.as_ref() == vcon.as_slice(),
        "the body read differs from the one stored",
    )?;
    println!("read {uuid} {} bytes", body.len());

    // Gzip: a gzipped body is stored as sent, and read back with its media type.
    let gzip_uuid = new_uuid()?;
    let mut zipper = GzEncoder::new(Vec::new(), Compression::default());
    zipper.write_all(&encode(&gzip_uuid, "gzip example"))?;
    let sent = zipper.finish()?;
    api.call(
        "POST",
        "/vcons",
        Some(&sent),
        &[("Content-Type", "application/vcon+gzip")],
    )?;
    let gzipped = api.call("GET", &format!("/vcons/{gzip_uuid}"), None, &[])?;
    let media = gzipped
        .headers()
        .get("Content-Type")
        .and_then(|media| media.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    expect(
        gzipped.bytes()?.as_ref() == sent.as_slice(),
        "the gzipped body read differs from the one stored",
    )?;
    expect(
        media == "application/vcon+gzip",
        "a gzipped vCon was not read back as application/vcon+gzip",
    )?;
    println!("gzip {gzip_uuid} read back as application/vcon+gzip");

    // Page: one vCon a page, from the first one's stored_at on, following
    // next until it is null. Each of the two must be seen exactly once. A
    // stored_at holds digits, '-', ':', '.', 'T' and 'Z'; only ':' needs
    // percent-encoding.
    let query = format!("/vcons?limit=1&since={}", stored_at.replace(':', "%3A"));
    let mut seen = Vec::new();
    let mut pages = 0;
    let mut path = query.clone();
    loop {
        let page = api.json(&api.key, &path)?;
        pages += 1;
        for v in page["vcons"].as_array().into_iter().flatten() {
            seen.push(v["uuid"].as_str().unwrap_or_default().to_owned());
        }
        match page["next"].as_str() {
            Some(cursor) => path = format!("{query}&cursor={cursor}"),
            None => break,
        }
    }
    for each in [&uuid, &gzip_uuid] {
        expect(
            seen.iter().filter(|s| *s == each).count() == 1,
            &format!("the pages did not list {each} exactly once"),
        )?;
    }
    println!("paged {} vCons in {pages} pages of 1", seen.len());

    // Replace: If-Match names the current version; the answer has the new
    // one. The first version's tag is then stale, and refused with 412.
    let replaced = encode(&uuid, "client example, replaced");
    let put = [
        ("If-Match", first_tag.as_str()),
        ("Content-Type", "application/vcon"),
    ];
    let tag = etag(&api.call("PUT", &format!("/vcons/{uuid}"), Some(&replaced), &put)?);
    expect(
        !tag.is_empty() && tag != first_tag,
        "a replace did not answer a new ETag",
    )?;
    let stale = api.refused("PUT", &format!("/vcons/{uuid}"), Some(&replaced), &put)?;
    expect(
        stale.ends_with(": 412 https://vcon-server.com/problems/precondition-failed"),
        &stale,
    )?;
    println!("replaced {uuid}, a stale If-Match refused with 412");

    // Legal hold: placed and released with the legal key, 204 each.
    for method in ["PUT", "DELETE"] {
        let held = api.send(
            &legal_key,
            method,
            &format!("/legal/vcons/{uuid}/hold"),
            None,
            &[],
        )?;
        expect(
            held.status() == StatusCode::NO_CONTENT,
            &format!("{method} hold did not answer 204"),
        )?;
    }
    println!("held {uuid} and released");

    // Audit: the tenant's log, read with the audit key, holds this
    // program's create, made by VCON_KEY, whose id and tenant GET /key gives.
    let me = api.json(&api.key, "/key")?;
    let key_id = me["id"].as_str().unwrap_or_default();
    let mut after = 0;
    loop {
        let page = api
            .send(
                &audit_key,
                "GET",
                &format!("/audit?after={after}&limit=1000"),
                None,
                &[],
            )?
            .text()?;
        let records = page
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<Vec<Value>, _>>()?;
        let found = records.iter().any(|r| {
            r["action"] == "create"
                && r["target"] == uuid.as_str()
                && r["actor"] == key_id
                && r["status"] == 201
        });
        if found {
            break;
        }
        expect(
            records.len() >= 1000,
            &format!("the audit log has no create of {uuid}"),
        )?;
        after = records
            .last()
            .and_then(|r| r["seq"].as_u64())
            .unwrap_or_default();
    }
    println!("audit create {uuid} by {key_id}");

    // Admin: the operator key lists every tenant, this key's among them.
    let tenant = me["tenant"].as_str().unwrap_or_default();
    let tenants = api.json(&operator_key, "/admin/tenants")?;
    expect(
        tenants
            .as_array()
            .is_some_and(|all| all.iter().any(|t| t["id"] == tenant)),
        &format!("the admin tenant list does not have {tenant}"),
    )?;
    println!("admin tenant {tenant} listed");

    // MCP: initialize, tools/list, and vcon_get, which answers the replaced
    // vCon's text between the untrusted-data markers, and its version.
    let init = api.mcp(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "vcon-client", "version": "1"},
        },
    }))?;
    expect(
        init["protocolVersion"] == "2025-06-18",
        "initialize did not agree to 2025-06-18",
    )?;
    api.mcp(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
    let tools = api.mcp(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))?;
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str())
        .collect();
    expect(names.contains(&"vcon_get"), "tools/list has no vcon_get")?;
    let got = api.mcp(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "vcon_get", "arguments": {"uuid": uuid}},
    }))?;
    expect(
        got["isError"] == false,
        &format!("vcon_get answered an error: {got}"),
    )?;
    let content = &got["structuredContent"];
    expect(
        format!("\"{}\"", content["version"].as_str().unwrap_or_default()) == tag,
        "vcon_get answered another version",
    )?;
    let text = format!(
        "\u{27e6}untrusted-vcon-data\u{27e7}{}\u{27e6}/untrusted-vcon-data\u{27e7}",
        String::from_utf8(replaced)?
    );
    expect(
        content["vcon"] == text.as_str(),
        "vcon_get answered another text",
    )?;
    let server = init["serverInfo"]["name"].as_str().unwrap_or_default();
    println!(
        "mcp vcon_get {uuid} from {server}, {} tools listed",
        names.len()
    );

    // Delete: 204, and each vCon is gone (410) from then on.
    for each in [&uuid, &gzip_uuid] {
        let deleted = api.call("DELETE", &format!("/vcons/{each}"), None, &[])?;
        expect(
            deleted.status() == StatusCode::NO_CONTENT,
            "a delete did not answer 204",
        )?;
        let gone = api.refused("GET", &format!("/vcons/{each}"), None, &[])?;
        expect(
            gone.ends_with(": 410 https://vcon-server.com/problems/gone"),
            &gone,
        )?;
    }
    println!("deleted {uuid} {gzip_uuid}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            let mut cause = failure.source();
            eprint!("{failure}");
            while let Some(inner) = cause {
                eprint!(": {inner}");
                cause = inner.source();
            }
            eprintln!();
            ExitCode::FAILURE
        }
    }
}
