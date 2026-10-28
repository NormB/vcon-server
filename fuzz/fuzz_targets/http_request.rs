//! Any request to the API's routes, served in-process without TLS; the
//! server, the input's layout and the checks are in `vcon_fuzz::http`.
#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use vcon_fuzz::http::World;

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
        World::new(&dir)
    })
}

fuzz_target!(|data: &[u8]| world().run(data));
