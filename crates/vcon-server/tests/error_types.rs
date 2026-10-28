//! The error types are well-behaved errors (Rust API Guidelines,
//! C-GOOD-ERR): each implements std::error::Error and is Send and Sync,
//! its Display says what went wrong in lower case with no trailing period,
//! and one that wraps an I/O error gives it as its source.

use std::error::Error;
use std::io;

use vcon_server::access::{AuthError, Scope};
use vcon_server::config::ConfigError;
use vcon_server::envelope::{KeyError, OpenError};
use vcon_server::query::BadQuery;

/// Compiles only for an error that can cross threads and be boxed.
fn is_good_error<E: Error + Send + Sync + 'static>() {}

#[test]
fn every_error_type_is_an_error_that_crosses_threads() {
    is_good_error::<AuthError>();
    is_good_error::<KeyError>();
    is_good_error::<OpenError>();
    is_good_error::<BadQuery>();
    is_good_error::<ConfigError>();
}

/// Each error as Display writes it.
fn displayed() -> Vec<(String, String)> {
    let errors: Vec<Box<dyn Error>> = vec![
        Box::new(AuthError::Missing),
        Box::new(AuthError::Forbidden(Scope::Read)),
        Box::new(AuthError::Malformed),
        Box::new(AuthError::Unknown),
        Box::new(AuthError::Revoked),
        Box::new(AuthError::Io),
        Box::new(KeyError::Memory(io::Error::other("no locked memory left"))),
        Box::new(KeyError::Read(io::Error::other("permission denied"))),
        Box::new(KeyError::WrongSize),
        Box::new(KeyError::Damaged),
        Box::new(OpenError),
        Box::new(BadQuery),
        Box::new(ConfigError("listen: missing".to_owned())),
    ];
    errors
        .iter()
        .map(|e| (format!("{e:?}"), e.to_string()))
        .collect()
}

#[test]
fn each_error_says_what_went_wrong_in_lower_case_with_no_period() {
    for (debug, text) in displayed() {
        let first = text.chars().next().expect("a message is never empty");
        assert!(
            !first.is_uppercase(),
            "{debug}: a message starts in lower case, to read inside a longer one: {text:?}"
        );
        assert!(
            !text.ends_with('.'),
            "{debug}: a message has no trailing period, to read inside a longer one: {text:?}"
        );
    }
}

#[test]
fn each_error_says_which_case_it_is() {
    let texts: Vec<String> = displayed().into_iter().map(|(_, text)| text).collect();
    let mut distinct = texts.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        texts.len(),
        "two cases with one message could not be told apart: {texts:?}"
    );
}

#[test]
fn a_forbidden_scope_is_named() {
    assert!(
        AuthError::Forbidden(Scope::Legal)
            .to_string()
            .contains("legal"),
        "the caller learns which scope it lacks"
    );
}

#[test]
fn a_key_error_from_io_gives_the_io_error_as_its_source() {
    for error in [
        KeyError::Memory(io::Error::other("no locked memory left")),
        KeyError::Read(io::Error::other("permission denied")),
    ] {
        let source = error.source().expect("an I/O error is wrapped");
        assert!(
            source.to_string() == "no locked memory left"
                || source.to_string() == "permission denied",
            "the source is the wrapped I/O error, unchanged: {source}"
        );
        assert!(
            !error.to_string().contains(&source.to_string()),
            "the source is given once, by source(), not repeated in Display: {error}"
        );
    }
    assert!(
        KeyError::WrongSize.source().is_none(),
        "a wrong-sized key file wraps no other error"
    );
}

#[test]
fn a_config_error_shows_its_text_as_it_is() {
    assert_eq!(
        ConfigError("listen: missing".to_owned()).to_string(),
        "listen: missing",
        "the text already names the key at fault"
    );
}
