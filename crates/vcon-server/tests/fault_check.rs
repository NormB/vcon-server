//! A planted fault for the CodeQL check; never merged.

#[test]
fn fault_check() {
    let _ = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build();
}
