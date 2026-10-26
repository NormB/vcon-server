//! The key rotation an operator runs, end to end, with the binary: a vCon
//! purged before the rotation cannot be recovered from a copy of the data
//! directory taken before the purge, once the old master key is gone.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;
use vcon_server::catalog::{self, CatalogError};
use vcon_server::clock::SystemClock;
use vcon_server::envelope::{self, Context, Key};
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId, VersionId};
use vcon_server::retention::Retention;
use vcon_server::store::{Caller, Created};
use vcon_server::tenants::Tenants;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Setup {
    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn key_file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .output()
            .expect("the binary runs")
    }

    fn ok(&self, args: &[&str]) -> Vec<Value> {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .expect("text")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect()
    }

    /// Points the configuration at the master key file `name`.
    fn use_master(&self, name: &str) {
        std::fs::write(
            &self.config,
            format!(
                "listen = \"127.0.0.1:1\"\ndata_dir = {:?}\nmaster_key = {:?}\n\
                 [tls]\ncert = \"unused.pem\"\nkey = \"unused.pem\"\n",
                self.data(),
                self.key_file(name)
            ),
        )
        .expect("configuration");
    }
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("data")).expect("data dir");
    std::fs::write(dir.path().join("old.key"), [0x61; 32]).expect("old master");
    let s = Setup {
        config: dir.path().join("vcon-server.toml"),
        dir,
    };
    s.use_master("old.key");
    s
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("list") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Stores a vCon in `tenant` as a producer would, and returns its version.
fn store_vcon(s: &Setup, master: &str, tenant: TenantId) -> VersionId {
    let tenants = Tenants::open(
        &s.data(),
        Key::load_master(&s.key_file(master)).expect("master"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .expect("opens");
    let body = format!(
        r#"{{"uuid": "{UUID}", "created_at": "2026-10-25T12:00:00Z", "subject": "to be purged", "parties": []}}"#
    );
    let found = vcon::inspect(
        body.as_bytes(),
        None,
        &vcon::Limits { inflated: 1 << 20 },
        &Default::default(),
    )
    .expect("a vCon");
    match tenants.store(tenant).expect("store").create(
        &Caller { key: KeyId([1; 8]) },
        &found,
        body.as_bytes(),
    ) {
        Ok(Created::New(version)) => version,
        other => panic!("{other:?}"),
    }
}

/// What the copy holds of the purged version, read with the keys of its
/// time: the wrapped data key and the sealed body.
struct Copied {
    wrapped_data_key: Vec<u8>,
    sealed: Vec<u8>,
}

/// Reads the copy with the old master key, as anyone who still had it
/// could, and checks that it does give the vCon back.
fn read_copy_with(copy: &Path, old_master: &Key, tenant: TenantId, version: VersionId) -> Copied {
    let server = catalog::open(
        &copy.join("server.sqlite"),
        &envelope::catalog_key(old_master).expect("key"),
        "",
    )
    .expect("the copy opens under the old master key");
    let wrapped: Vec<u8> = server
        .query_row(
            "SELECT wrapped_tenant_key FROM tenant WHERE id = ?1",
            [tenant.0],
            |r| r.get(0),
        )
        .expect("the tenant's row");
    let tenant_key = envelope::unwrap(old_master, &Context::Tenant { tenant }, &wrapped)
        .expect("the old tenant key");
    let dir = copy.join("tenants").join(tenant.to_string());
    let catalog = catalog::open(
        &dir.join("catalog.sqlite"),
        &envelope::catalog_key(&tenant_key).expect("key"),
        "",
    )
    .expect("the tenant's old catalog");
    let wrapped_data_key: Vec<u8> = catalog
        .query_row(
            "SELECT wrapped_key FROM version WHERE version = ?1",
            [version.0],
            |r| r.get(0),
        )
        .expect("the version's row");
    let ctx = Context::Version {
        tenant,
        uuid: Uuid::parse_str(UUID).expect("uuid"),
        version,
    };
    let data_key = envelope::unwrap(&tenant_key, &ctx, &wrapped_data_key).expect("the data key");
    let sealed = std::fs::read(dir.join("objects").join(version.to_string())).expect("sealed");
    let body = envelope::open(&data_key, &ctx, &sealed).expect("the body opens");
    assert!(
        String::from_utf8_lossy(&body).contains("to be purged"),
        "the copy really holds the vCon"
    );
    Copied {
        wrapped_data_key,
        sealed,
    }
}

#[test]
fn a_vcon_purged_before_a_rotation_cannot_be_recovered_from_an_old_copy() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let tenant = TenantId::parse(&tenant).expect("tenant id");
    let version = store_vcon(&s, "old.key", tenant);

    // A backup, taken while the vCon was kept.
    let copy = s.dir.path().join("backup");
    copy_dir(&s.data(), &copy);

    // The vCon is deleted under a zero retention period: purged at once.
    s.ok(&["tenant", "set-retention", &tenant.to_string(), "0d"]);
    {
        let tenants = Tenants::open(
            &s.data(),
            Key::load_master(&s.key_file("old.key")).expect("master"),
            Arc::new(OsFs),
            Arc::new(SystemClock),
        )
        .expect("opens");
        tenants
            .store(tenant)
            .expect("store")
            .delete(
                &Caller { key: KeyId([1; 8]) },
                Uuid::parse_str(UUID).expect("uuid"),
                Retention::Days(0),
            )
            .expect("deleted and purged");
    }

    // Until the rotation, anyone with the old master key reads the copy.
    let copied = read_copy_with(
        &copy,
        &Key::load_master(&s.key_file("old.key")).expect("old"),
        tenant,
        version,
    );

    // The rotation, as the operator guide gives it.
    let checked = s.ok(&["check"]);
    assert_eq!(checked[0]["purges_awaiting_rotation"], 1);
    s.ok(&["rekey", "tenant", &tenant.to_string()]);
    assert_eq!(s.ok(&["check"])[0]["purges_awaiting_rotation"], 0);
    std::fs::write(s.key_file("new.key"), [0x62; 32]).expect("new master");
    let new_master_path = s.key_file("new.key");
    s.ok(&[
        "rekey",
        "master",
        "--new-master",
        new_master_path.to_str().expect("path"),
    ]);
    s.use_master("new.key");
    std::fs::remove_file(s.key_file("old.key")).expect("the old master key destroyed");
    assert_eq!(s.ok(&["check"]).last().expect("summary")["problems"], 0);

    // Every key that still exists, tried on the copy.
    let new_master = Key::load_master(&s.key_file("new.key")).expect("new master");
    assert!(matches!(
        catalog::open(
            &copy.join("server.sqlite"),
            &envelope::catalog_key(&new_master).expect("key"),
            ""
        ),
        Err(CatalogError::WrongKey)
    ));
    let live_server = catalog::open(
        &s.data().join("server.sqlite"),
        &envelope::catalog_key(&new_master).expect("key"),
        "",
    )
    .expect("the live registry opens under the new master");
    let live_wrapped: Vec<u8> = live_server
        .query_row(
            "SELECT wrapped_tenant_key FROM tenant WHERE id = ?1",
            [tenant.0],
            |r| r.get(0),
        )
        .expect("the tenant's row");
    drop(live_server);
    let tenant_key = envelope::unwrap(&new_master, &Context::Tenant { tenant }, &live_wrapped)
        .expect("the current tenant key");
    let old_catalog = copy
        .join("tenants")
        .join(tenant.to_string())
        .join("catalog.sqlite");
    for key in [&new_master, &tenant_key] {
        assert!(matches!(
            catalog::open(&old_catalog, &envelope::catalog_key(key).expect("key"), ""),
            Err(CatalogError::WrongKey)
        ));
    }
    let ctx = Context::Version {
        tenant,
        uuid: Uuid::parse_str(UUID).expect("uuid"),
        version,
    };
    for key in [&new_master, &tenant_key] {
        assert!(envelope::unwrap(key, &ctx, &copied.wrapped_data_key).is_err());
        assert!(envelope::open(key, &ctx, &copied.sealed).is_err());
    }
    // And the live data no longer holds it at all.
    assert!(
        !s.data()
            .join("tenants")
            .join(tenant.to_string())
            .join("objects")
            .join(version.to_string())
            .exists()
    );
}
