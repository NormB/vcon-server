//! Every on-disk format this build writes, with its version: the one list.
//!
//! The page website/content/docs/on-disk-format.md lists each entry, and
//! tests/formats.rs holds the page to [`FORMATS`] and opens the data each
//! released binary wrote (tests/data/formats). Each version is read from
//! the constant the format's own code uses, except the PostgreSQL schema's,
//! which is defined here and used by that code, since the module that
//! migrates it is built only with the `postgres` feature.

/// One on-disk format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// Its name on the page, in the formats table and as a column of the
    /// release table.
    pub id: &'static str,
    /// The version this build writes, as the page states it.
    pub version: &'static str,
}

/// The version of the PostgreSQL catalog's schema this build makes and
/// reads; the migrations' count, which a test of that module holds equal.
pub const POSTGRES_SCHEMA: i32 = 14;

/// The digits of a small number, at compile time, for [`FORMATS`].
const fn digits(n: i64) -> &'static str {
    match n {
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        10 => "10",
        11 => "11",
        12 => "12",
        13 => "13",
        14 => "14",
        15 => "15",
        16 => "16",
        _ => panic!("add the number to formats::digits"),
    }
}

/// Every on-disk format, in the order of the page's table.
pub const FORMATS: &[Format] = &[
    Format {
        id: "catalog",
        version: digits(crate::catalog::SCHEMA_VERSION),
    },
    Format {
        id: "postgres",
        version: digits(POSTGRES_SCHEMA as i64),
    },
    Format {
        id: "envelope",
        version: digits(crate::envelope::FORMAT as i64),
    },
    Format {
        id: "parquet-storage",
        version: crate::store::manifest::FORMAT,
    },
    Format {
        id: "export-manifest",
        version: digits(crate::export::MANIFEST_FORMAT as i64),
    },
    Format {
        id: "export-columns",
        version: crate::export::columns::SCHEMA,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: &str) -> &'static str {
        FORMATS
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("{id} is listed"))
            .version
    }

    #[test]
    fn each_version_is_its_formats_own_constant() {
        assert_eq!(
            version("catalog"),
            crate::catalog::SCHEMA_VERSION.to_string(),
            "catalog"
        );
        assert_eq!(
            version("envelope"),
            crate::envelope::FORMAT.to_string(),
            "envelope"
        );
        assert_eq!(
            version("parquet-storage"),
            crate::store::manifest::FORMAT,
            "parquet"
        );
        assert_eq!(
            version("export-manifest"),
            crate::export::MANIFEST_FORMAT.to_string(),
            "export manifest"
        );
        assert_eq!(
            version("export-columns"),
            crate::export::columns::SCHEMA,
            "export columns"
        );
        assert_eq!(version("postgres"), POSTGRES_SCHEMA.to_string(), "postgres");
    }

    #[test]
    fn digits_writes_each_number_it_knows() {
        for n in 1..=16 {
            assert_eq!(digits(n), n.to_string(), "{n}");
        }
    }

    #[test]
    #[should_panic(expected = "add the number")]
    fn digits_refuses_a_number_it_does_not_know() {
        let _ = digits(17);
    }
}
