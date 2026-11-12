// The parts of a session a request carries, by kind: what must work, what
// must be refused, and the limits and odd inputs.

use axum::http::{HeaderMap, HeaderValue};

use super::{
    COOKIE, Cookie, Origin, amr_text, clear_cookie, cookie, csrf_token, from_origin, hash,
    new_secret, same, set_cookie, well_formed,
};

fn headers(of: &[(&'static str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in of {
        headers.append(*name, HeaderValue::from_str(value).expect("a header value"));
    }
    headers
}

fn origin() -> Origin {
    Origin::parse("https://vcon.example.com").expect("an origin")
}

mod works {
    use super::*;

    #[test]
    fn a_new_secret_is_256_random_bits_in_43_url_safe_characters() {
        let (one, two) = (new_secret(), new_secret());
        assert_eq!(one.len(), 43, "32 bytes in base64url with no padding");
        assert!(well_formed(&one), "a new secret is well formed: {one}");
        assert_ne!(one, two, "two secrets differ");
    }

    #[test]
    fn the_cookie_is_host_bound_secure_and_kept_from_page_script() {
        let id = new_secret();
        let set = set_cookie(&id);
        assert_eq!(
            set.to_str().expect("text"),
            format!("__Host-vcs_session={id}; Path=/; Secure; HttpOnly; SameSite=Strict"),
            "the whole cookie, with no Domain and no lifetime"
        );
    }

    #[test]
    fn the_cookie_is_cleared_by_the_same_attributes_and_no_lifetime_left() {
        assert_eq!(
            clear_cookie().to_str().expect("text"),
            "__Host-vcs_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
            "a cookie is removed by one of its own name and path"
        );
    }

    #[test]
    fn a_request_s_cookie_is_read_among_others() {
        let id = new_secret();
        let sent = headers(&[("cookie", &format!("theme=dark; {COOKIE}={id}; lang=en"))]);
        assert_eq!(cookie(&sent), Cookie::Id(&id), "found between two others");
    }

    #[test]
    fn a_request_s_cookie_is_read_from_a_second_cookie_header() {
        let id = new_secret();
        let sent = headers(&[
            ("cookie", "theme=dark"),
            ("cookie", &format!("{COOKIE}={id}")),
        ]);
        assert_eq!(
            cookie(&sent),
            Cookie::Id(&id),
            "HTTP/2 may split the header"
        );
    }

    #[test]
    fn the_csrf_token_of_a_session_is_the_same_each_time_and_is_not_its_id() {
        let id = new_secret();
        let token = csrf_token(&id);
        assert_eq!(token, csrf_token(&id), "one session, one token");
        assert_ne!(token, id, "the token does not show the id");
        assert_ne!(hash(&token), hash(&id), "nor does its hash");
        assert!(well_formed(&token), "43 url-safe characters: {token}");
        assert_ne!(
            token,
            csrf_token(&new_secret()),
            "another session's differs"
        );
    }

    #[test]
    fn the_csrf_token_cannot_be_made_from_what_the_registry_keeps() {
        use base64::Engine;
        let id = new_secret();
        let kept = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash(&id));
        assert_ne!(
            csrf_token(&id),
            kept,
            "the token is not the hash the session's row is found by"
        );
    }

    #[test]
    fn a_hash_is_the_sha_256_of_the_text() {
        // The test vector of FIPS 180-2 for "abc".
        let expected = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(hash("abc"), expected, "SHA-256 of abc");
    }

    #[test]
    fn an_origin_is_https_a_host_and_perhaps_a_port() {
        for text in [
            "https://vcon.example.com",
            "https://vcon.example.com:8443",
            "https://localhost:8443",
            "https://192.0.2.7:8443",
            "https://[2001:db8::7]:8443",
        ] {
            let origin = Origin::parse(text).expect(text);
            assert_eq!(origin.as_str(), text, "kept as written");
        }
        assert_eq!(origin().host(), "vcon.example.com", "the host alone");
        let with_port = Origin::parse("https://[2001:db8::7]:8443").expect("an origin");
        assert_eq!(with_port.host(), "[2001:db8::7]", "without the port");
    }

    #[test]
    fn a_request_from_the_listener_s_origin_passes() {
        let sent = headers(&[("origin", "https://vcon.example.com")]);
        assert!(
            from_origin(&sent, Some(&origin()), false),
            "the same origin"
        );
    }

    #[test]
    fn the_hand_out_passes_on_a_same_origin_fetch_with_or_without_origin() {
        let fetched = headers(&[("sec-fetch-site", "same-origin")]);
        assert!(
            from_origin(&fetched, Some(&origin()), true),
            "a same-origin GET"
        );
        let both = headers(&[
            ("sec-fetch-site", "same-origin"),
            ("origin", "https://vcon.example.com"),
        ]);
        assert!(
            from_origin(&both, Some(&origin()), true),
            "with its Origin too"
        );
    }

    #[test]
    fn authentication_methods_are_written_as_one_text() {
        assert_eq!(amr_text(&["hwk"]), Some("hwk".to_owned()), "one method");
        assert_eq!(
            amr_text(&["pwd", "otp", "mfa"]),
            Some("pwd otp mfa".to_owned()),
            "in the order given"
        );
    }

    #[test]
    fn equal_bytes_are_the_same_and_others_are_not() {
        assert!(same(&hash("a"), &hash("a")), "equal");
        assert!(!same(&hash("a"), &hash("b")), "different");
    }
}

mod refused {
    use super::*;

    #[test]
    fn a_request_with_no_cookie_header_has_no_session_cookie() {
        assert_eq!(cookie(&HeaderMap::new()), Cookie::Absent, "no header");
        let others = headers(&[("cookie", "theme=dark; lang=en")]);
        assert_eq!(cookie(&others), Cookie::Absent, "other cookies only");
    }

    #[test]
    fn two_session_cookies_are_refused_whichever_is_first() {
        let (one, two) = (new_secret(), new_secret());
        let sent = headers(&[("cookie", &format!("{COOKIE}={one}; {COOKIE}={two}"))]);
        assert_eq!(cookie(&sent), Cookie::Bad, "two in one header");
        let split = headers(&[
            ("cookie", &format!("{COOKIE}={one}")),
            ("cookie", &format!("{COOKIE}={one}")),
        ]);
        assert_eq!(
            cookie(&split),
            Cookie::Bad,
            "two in two headers, even alike"
        );
    }

    #[test]
    fn a_cookie_value_that_no_session_id_has_is_refused_unread() {
        let id = new_secret();
        for value in [
            String::new(),
            id[..42].to_owned(),
            format!("{id}A"),
            format!("{}=", &id[..42]),
            format!("{}+", &id[..42]),
            format!("\"{id}\""),
        ] {
            let sent = headers(&[("cookie", &format!("{COOKIE}={value}"))]);
            assert_eq!(cookie(&sent), Cookie::Bad, "{value:?}");
        }
    }

    #[test]
    fn a_cookie_header_that_is_not_text_is_refused() {
        let mut sent = HeaderMap::new();
        let bytes = [b"__Host-vcs_session=".as_slice(), &[0xff, 0xfe]].concat();
        sent.insert(
            "cookie",
            HeaderValue::from_bytes(&bytes).expect("opaque bytes"),
        );
        assert_eq!(cookie(&sent), Cookie::Bad, "not UTF-8");
    }

    #[test]
    fn an_origin_that_is_not_https_and_a_host_is_refused() {
        for text in [
            "",
            "http://vcon.example.com",
            "vcon.example.com",
            "https://",
            "https://vcon.example.com/",
            "https://vcon.example.com/console",
            "https://vcon.example.com?x=1",
            "https://vcon.example.com#top",
            "https://user@vcon.example.com",
            "https://vcon.example.com:",
            "https://vcon.example.com:0",
            "https://vcon.example.com:65536",
            "https://vcon.example.com:8443:1",
            "https://VCON.example.com",
            "HTTPS://vcon.example.com",
            "https://vcon example.com",
            "https://[2001:db8::7",
            "null",
        ] {
            assert_eq!(Origin::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_request_from_another_origin_is_refused() {
        for other in [
            "https://evil.example.net",
            "https://vcon.example.com:8443",
            "http://vcon.example.com",
            "https://vcon.example.com.evil.example.net",
            "https://vcon.example.com/",
            "null",
            "",
        ] {
            let sent = headers(&[("origin", other)]);
            assert!(!from_origin(&sent, Some(&origin()), false), "{other:?}");
        }
    }

    #[test]
    fn a_request_with_no_origin_header_is_refused_unless_it_is_the_hand_out() {
        let none = HeaderMap::new();
        assert!(!from_origin(&none, Some(&origin()), false), "no Origin");
        let fetched = headers(&[("sec-fetch-site", "same-origin")]);
        assert!(
            !from_origin(&fetched, Some(&origin()), false),
            "Sec-Fetch-Site does not stand in for Origin on any other request"
        );
    }

    #[test]
    fn the_hand_out_is_refused_to_a_fetch_that_is_not_same_origin() {
        for site in ["same-site", "cross-site", "none", "Same-Origin", ""] {
            let sent = headers(&[("sec-fetch-site", site)]);
            assert!(!from_origin(&sent, Some(&origin()), true), "{site:?}");
        }
        assert!(
            !from_origin(&HeaderMap::new(), Some(&origin()), true),
            "no Sec-Fetch-Site: not a browser's fetch"
        );
        let own = headers(&[("origin", "https://vcon.example.com")]);
        assert!(
            !from_origin(&own, Some(&origin()), true),
            "Origin does not stand in for Sec-Fetch-Site"
        );
    }

    #[test]
    fn the_hand_out_is_refused_when_its_origin_header_is_another_s() {
        let sent = headers(&[
            ("sec-fetch-site", "same-origin"),
            ("origin", "https://evil.example.net"),
        ]);
        assert!(!from_origin(&sent, Some(&origin()), true), "a forged pair");
    }

    #[test]
    fn a_listener_with_no_public_origin_passes_no_request() {
        let sent = headers(&[
            ("origin", "https://vcon.example.com"),
            ("sec-fetch-site", "same-origin"),
        ]);
        assert!(!from_origin(&sent, None, false), "no origin to equal");
        assert!(!from_origin(&sent, None, true), "nor for the hand-out");
    }

    #[test]
    fn authentication_methods_that_cannot_be_kept_are_refused() {
        let long = "a".repeat(33);
        let nine = ["a"; 9];
        let cases: [&[&str]; 7] = [
            &[],
            &[""],
            &["two words"],
            &["pwd", "pwd"],
            &[&long],
            &nine,
            &["PWD"],
        ];
        for methods in cases {
            assert_eq!(amr_text(methods), None, "{methods:?}");
        }
    }
}

mod limits {
    use super::*;

    #[test]
    fn a_cookie_name_is_matched_whole_and_in_its_case() {
        let id = new_secret();
        for name in [
            "__host-vcs_session",
            "__Host-vcs_session2",
            "x__Host-vcs_session",
            "__Secure-vcs_session",
            "vcs_session",
        ] {
            let sent = headers(&[("cookie", &format!("{name}={id}"))]);
            assert_eq!(cookie(&sent), Cookie::Absent, "{name}");
        }
    }

    #[test]
    fn white_space_around_a_cookie_pair_is_not_part_of_it() {
        let id = new_secret();
        let sent = headers(&[("cookie", &format!("a=b;   {COOKIE}={id}  ;c=d"))]);
        assert_eq!(cookie(&sent), Cookie::Id(&id), "trimmed");
    }

    #[test]
    fn a_cookie_pair_with_no_equals_sign_is_not_the_session_cookie() {
        let sent = headers(&[("cookie", COOKIE)]);
        assert_eq!(cookie(&sent), Cookie::Absent, "a name alone");
    }

    #[test]
    fn two_origin_headers_are_refused_even_when_both_are_the_listener_s() {
        let sent = headers(&[
            ("origin", "https://vcon.example.com"),
            ("origin", "https://vcon.example.com"),
        ]);
        assert!(
            !from_origin(&sent, Some(&origin()), false),
            "one is expected"
        );
        let fetched = headers(&[
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-site", "same-origin"),
        ]);
        assert!(
            !from_origin(&fetched, Some(&origin()), true),
            "one is expected"
        );
    }

    #[test]
    fn an_origin_s_port_is_1_to_65535_and_its_host_at_most_253_bytes() {
        assert!(Origin::parse("https://a:1").is_some(), "port 1");
        assert!(Origin::parse("https://a:65535").is_some(), "port 65535");
        let host = "a".repeat(253);
        assert!(
            Origin::parse(&format!("https://{host}")).is_some(),
            "253 bytes"
        );
        let host = "a".repeat(254);
        assert!(
            Origin::parse(&format!("https://{host}")).is_none(),
            "254 bytes"
        );
    }

    #[test]
    fn eight_authentication_methods_of_32_characters_are_kept() {
        let names: Vec<String> = (0..8).map(|n| format!("{n}{}", "a".repeat(31))).collect();
        let methods: Vec<&str> = names.iter().map(String::as_str).collect();
        assert_eq!(amr_text(&methods), Some(names.join(" ")), "the most kept");
    }

    #[test]
    fn slices_of_different_lengths_are_not_the_same() {
        assert!(!same(&[1, 2, 3], &[1, 2]), "a prefix is not equal");
        assert!(same(&[], &[]), "two empty slices are");
    }

    #[test]
    fn the_secret_alphabet_is_the_url_safe_one() {
        assert!(well_formed(&"A".repeat(43)), "letters");
        assert!(
            well_formed(&"-_09azAZ".repeat(6)[..43]),
            "the whole alphabet"
        );
        assert!(!well_formed(&"A".repeat(42)), "one short");
        assert!(!well_formed(&"A".repeat(44)), "one long");
        assert!(
            !well_formed(&format!("{}/", "A".repeat(42))),
            "the plain alphabet's /"
        );
        assert!(
            !well_formed(&format!("{}é", "A".repeat(41))),
            "43 bytes, not ASCII"
        );
    }
}
