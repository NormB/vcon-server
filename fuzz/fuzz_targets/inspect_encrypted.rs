//! Any JSON text as the `unprotected` member of an encrypted vCon, the
//! only part of a JWE the server reads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon::Form;
use vcon_fuzz::{LIMITS, jwe, supported};

fuzz_target!(|unprotected: &[u8]| {
    let found = vcon::read::inspect(&jwe(unprotected), None, &LIMITS, &supported());
    if let Ok(found) = found {
        assert_eq!(found.form, Form::Encrypted);
    }
});
