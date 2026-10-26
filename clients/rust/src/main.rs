//! Stores a vCon, finds it in the list, reads it back, and deletes it, with
//! reqwest's blocking client.
//!
//! Set `VCON_URL` (`https://vcon.example.com`), `VCON_KEY` (a key with the
//! write, read and delete scopes) and, for a certificate your system does
//! not trust, `VCON_CA` (its PEM file).

use std::io::Read;
use std::process::ExitCode;

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
    /// Sends one request. An answer of 400 or more is a problem document,
    /// returned as an error with its type.
    fn call(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<Response, Failure> {
        let method = reqwest::Method::from_bytes(method.as_bytes())?;
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.url))
            .bearer_auth(&self.key);
        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/vcon")
                .body(body.to_vec());
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
    let uuid = new_uuid()?;
    // serde_json writes an object's keys sorted.
    let vcon = json!({
        "created_at": "2026-10-25T12:00:00Z",
        "parties": [],
        "subject": "client example",
        "uuid": uuid,
    })
    .to_string()
    .into_bytes();

    // Store: 201, with the new version in ETag.
    let stored = api.call("POST", "/vcons", Some(&vcon))?;
    expect(
        stored.status() == StatusCode::CREATED,
        "a store did not answer 201",
    )?;
    println!("stored {uuid}");

    // List: the newest vCon comes first.
    let page: Value = serde_json::from_slice(&api.call("GET", "/vcons?limit=10", None)?.bytes()?)?;
    let listed = page["vcons"]
        .as_array()
        .is_some_and(|vcons| vcons.iter().any(|v| v["uuid"] == uuid.as_str()));
    expect(listed, "the stored vCon is not in the list")?;
    println!("listed {uuid}");

    // Read: the body comes back exactly as stored.
    let body = api.call("GET", &format!("/vcons/{uuid}"), None)?.bytes()?;
    expect(
        body.as_ref() == vcon.as_slice(),
        "the body read differs from the one stored",
    )?;
    println!("read {uuid} {} bytes", body.len());

    // Delete: 204, and the vCon is gone (410) from then on.
    let deleted = api.call("DELETE", &format!("/vcons/{uuid}"), None)?;
    expect(
        deleted.status() == StatusCode::NO_CONTENT,
        "a delete did not answer 204",
    )?;
    let gone = api
        .http
        .get(format!("{}/vcons/{uuid}", api.url))
        .bearer_auth(&api.key)
        .send()?;
    expect(
        gone.status() == StatusCode::GONE,
        "a deleted vCon was not 410",
    )?;
    println!("deleted {uuid}");
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
