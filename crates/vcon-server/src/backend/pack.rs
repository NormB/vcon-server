//! A pack file: sealed versions of one tenant and one store date, written
//! in the export's Parquet format (`export::columns`, level `none`), and
//! read back one row at a time.
//!
//! A row is read in two steps. The footer and its offset index are read
//! once from the end of the file; they say where each row's `sealed` page
//! is, since every value is a page of its own. Then the one page that
//! holds the row is fetched by a ranged read and decoded from its own
//! bytes alone. A file never changes once written, so a footer read once
//! stays right for as long as the file exists.
//!
//! Nothing here checks that a page belongs to the version asked for: a
//! wrong place gives another row's value, or none. The sealed value is the
//! version's AES-GCM blob, bound to its tenant, uuid and version, so the
//! Store's open of it is the check, and a wrong place is `Damaged`.

use std::ops::Range;
use std::sync::Arc;

use bytes::Bytes;
use parquet::basic::{Encoding, Type};
use parquet::column::page::{Page, PageReader};
use parquet::errors::ParquetError;
use parquet::file::metadata::{PageIndexPolicy, ParquetMetaData, ParquetMetaDataReader};
use parquet::file::reader::{ChunkReader, Length};
use parquet::file::serialized_reader::SerializedPageReader;

pub(crate) use crate::export::columns::Place;
use crate::export::columns::{SCHEMA, SCHEMA_KEY};
use crate::ids::FileId;

/// How many bytes from the end of a file are read first for its footer;
/// a footer that needs more is read again at its own length.
pub(crate) const TAIL: u64 = 64 << 10;

/// The name of a pack file under its tenant's directory.
pub(crate) fn name(date: &str, file: FileId) -> String {
    format!("parquet/date={date}/part-{file}.parquet")
}

/// The date and file id of a name `name` writes; None for any other name.
pub(crate) fn parse_name(text: &str) -> Option<(String, FileId)> {
    let rest = text.strip_prefix("parquet/date=")?;
    let (date, file) = rest.split_once("/part-")?;
    let file = FileId::parse(file.strip_suffix(".parquet")?)?;
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let mut parts = date.split('-');
    let whole = matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(y), Some(m), Some(d), None) if digits(y, 4) && digits(m, 2) && digits(d, 2)
    );
    whole.then(|| (date.to_owned(), file))
}

/// The UTC date of a store time in nanoseconds, as a pack file's name and
/// an export's partition write it; None for a time this build cannot hold.
pub(crate) fn date_of(stored_at: i64) -> Option<String> {
    let at = crate::clock::from_nanos(stored_at)?;
    let (year, month, day) = (at.year(), u8::from(at.month()), at.day());
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// What reading a footer from the end of a file gave.
#[derive(Debug)]
pub(crate) enum Parsed {
    Footer(Footer),
    /// The footer needs this many bytes from the end of the file.
    More(u64),
}

/// A pack file's footer and offset index.
#[derive(Debug)]
pub(crate) struct Footer {
    metadata: ParquetMetaData,
    /// The index of the `sealed` column.
    sealed: usize,
    /// The file's length in bytes.
    size: u64,
}

impl Footer {
    /// Reads the footer and offset index from `tail`, the last bytes of a
    /// file of `size` bytes. None: the bytes are not a pack file's end.
    pub(crate) fn parse(tail: Vec<u8>, size: u64) -> Option<Parsed> {
        let tail = Bytes::from(tail);
        if tail.len() as u64 > size {
            return None;
        }
        let mut reader = ParquetMetaDataReader::new()
            .with_column_index_policy(PageIndexPolicy::Skip)
            .with_offset_index_policy(PageIndexPolicy::Required);
        match reader.try_parse_sized(&tail, size) {
            Ok(()) => {}
            Err(ParquetError::NeedMoreData(n)) => {
                let n = u64::try_from(n).ok()?;
                return (n > tail.len() as u64 && n <= size).then_some(Parsed::More(n));
            }
            Err(_) => return None,
        }
        let metadata = reader.finish().ok()?;
        let written = metadata
            .file_metadata()
            .key_value_metadata()
            .into_iter()
            .flatten()
            .any(|kv| kv.key == SCHEMA_KEY && kv.value.as_deref() == Some(SCHEMA));
        let columns = metadata.file_metadata().schema_descr();
        let sealed = (0..columns.num_columns()).find(|&i| columns.column(i).name() == "sealed")?;
        let column = columns.column(sealed);
        // A compressed page fails later, when its reader finds no codec
        // compiled in to decompress it.
        let plain = column.physical_type() == Type::BYTE_ARRAY
            && column.max_def_level() == 0
            && column.max_rep_level() == 0;
        (written && plain).then_some(Parsed::Footer(Footer {
            metadata,
            sealed,
            size,
        }))
    }

    /// The bytes of the file that hold `place`'s `sealed` page, its header
    /// included. None: the file has no such row.
    pub(crate) fn page(&self, place: Place) -> Option<Range<u64>> {
        let location = self.location(place)?;
        let start = u64::try_from(location.offset).ok()?;
        let len = u64::try_from(location.compressed_page_size).ok()?;
        Some(start..start.checked_add(len)?)
    }

    /// The `sealed` value of `place`, decoded from `page`, the bytes
    /// `page` named. None: they are not a page of one value.
    pub(crate) fn sealed(&self, place: Place, page: Vec<u8>) -> Option<Vec<u8>> {
        let range = self.page(place)?;
        let group = usize::try_from(place.group).ok()?;
        let row = usize::try_from(place.row).ok()?;
        let index = self.metadata.page_index_for_row_group(group);
        let locations = index.page_locations(self.sealed)?.clone();
        let chunk = self.metadata.row_group(group).column(self.sealed);
        let rows = usize::try_from(self.metadata.row_group(group).num_rows()).ok()?;
        let one = Arc::new(OnePage {
            start: range.start,
            bytes: Bytes::from(page),
            size: self.size,
        });
        let mut pages = SerializedPageReader::new(one, chunk, rows, Some(locations)).ok()?;
        for _ in 0..row {
            pages.skip_next_page().ok()?;
        }
        let Page::DataPage {
            buf,
            num_values: 1,
            encoding: Encoding::PLAIN,
            ..
        } = pages.get_next_page().ok()??
        else {
            return None;
        };
        // PLAIN BYTE_ARRAY: a 4-byte little-endian length, then the value.
        let (length, value) = buf.split_at_checked(4)?;
        let length = u32::from_le_bytes(length.try_into().ok()?);
        (usize::try_from(length).ok()? == value.len()).then(|| value.to_vec())
    }

    /// The offset index entry of `place`'s `sealed` page.
    fn location(
        &self,
        place: Place,
    ) -> Option<parquet::file::page_index::offset_index::PageLocation> {
        let group = usize::try_from(place.group).ok()?;
        if group >= self.metadata.num_row_groups() {
            return None;
        }
        let index = self.metadata.page_index_for_row_group(group);
        let locations = index.page_locations(self.sealed)?;
        let location = locations.get(usize::try_from(place.row).ok()?)?;
        // One value per page: page i starts at row i.
        (location.first_row_index == i64::from(place.row)).then(|| location.clone())
    }
}

/// The bytes of one page at their place in a file, read as the file: any
/// read of other bytes fails, so a page decoded through it was decoded from
/// its own bytes alone.
struct OnePage {
    start: u64,
    bytes: Bytes,
    size: u64,
}

impl OnePage {
    fn slice(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        let outside = || ParquetError::General("a read outside the page".to_owned());
        let from = usize::try_from(start.checked_sub(self.start).ok_or_else(outside)?)
            .map_err(|_| outside())?;
        let to = from.checked_add(length).ok_or_else(outside)?;
        if to > self.bytes.len() {
            return Err(outside());
        }
        Ok(self.bytes.slice(from..to))
    }
}

impl Length for OnePage {
    fn len(&self) -> u64 {
        self.size
    }
}

impl ChunkReader for OnePage {
    type T = bytes::buf::Reader<Bytes>;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        // A reader runs to the end of the page; one from before or past it
        // fails in `slice`.
        let from = usize::try_from(start.saturating_sub(self.start)).unwrap_or(usize::MAX);
        let length = self.bytes.len().saturating_sub(from);
        Ok(bytes::Buf::reader(self.slice(start, length)?))
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        self.slice(start, length)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::export::Metadata;
    use crate::export::columns::{About, File, Row, Sizes};
    use crate::ids::VersionId;

    /// A row whose sealed value is `n` repeated, after bytes that are not
    /// UTF-8, `n` bytes longer than the row before it.
    fn row(n: u8) -> Row {
        let mut sealed = vec![0x1f, 0x8b, 0x00, 0xff];
        sealed.extend(std::iter::repeat_n(n, 40 + usize::from(n)));
        Row {
            version: VersionId([n; 16]),
            stored_at: 1_762_300_000_000_000_000 + i64::from(n),
            sealed,
            about: About::default(),
        }
    }

    /// The version id of each row of a pack file, in order.
    pub(crate) fn versions_in(bytes: &[u8]) -> Vec<VersionId> {
        crate::export::columns::tests::read(bytes)
            .into_iter()
            .map(|row| {
                let (_, value) = row
                    .into_iter()
                    .find(|(name, _)| name == "version")
                    .expect("a version column");
                match value {
                    parquet::record::Field::Bytes(b) => {
                        VersionId(b.data().try_into().expect("16 bytes"))
                    }
                    other => panic!("a version id, not {other:?}"),
                }
            })
            .collect()
    }

    /// `rows` written to a file of row groups of about `group` bytes, and
    /// each row's place.
    pub(crate) fn packed(rows: &[Row], group: usize) -> (Vec<u8>, Vec<Place>) {
        let sizes = Sizes {
            row_group: group,
            file: usize::MAX,
        };
        let mut file = File::new(Metadata::None, sizes).expect("a new file");
        let places = rows
            .iter()
            .map(|r| file.push(r.clone()).expect("adds a row"))
            .collect();
        (file.finish().expect("the file"), places)
    }

    /// The footer of `bytes`, read from as few bytes of its end as it
    /// needs: first 8, then each length it asks for, each longer than the
    /// last and shorter than the file: the footer's, then its offset
    /// index's, which is written before it.
    pub(crate) fn footer(bytes: &[u8]) -> Footer {
        let size = bytes.len() as u64;
        let tail = |n: u64| bytes[bytes.len() - n as usize..].to_vec();
        let mut asked = vec![8];
        loop {
            let n = *asked.last().expect("a length");
            match Footer::parse(tail(n), size).expect("a pack file's end") {
                Parsed::Footer(footer) => return footer,
                Parsed::More(more) => {
                    assert!(more > n && more < size, "{asked:?}, then {more}");
                    asked.push(more);
                    assert!(asked.len() <= 3, "the footer, then its index: {asked:?}");
                }
            }
        }
    }

    /// Each row's place names its row group and its row in it, and each
    /// row reads back from its own page's bytes alone, byte for byte; the
    /// page holds no other row's value.
    #[test]
    fn each_row_reads_back_from_its_own_page_alone() {
        let rows: Vec<Row> = (1..=9).map(row).collect();
        let (bytes, places) = packed(&rows, 120);
        let groups: Vec<u32> = places.iter().map(|p| p.group).collect();
        assert!(
            groups.iter().max() > Some(&1),
            "several row groups: {groups:?}"
        );
        let mut expected = Place { group: 0, row: 0 };
        for place in &places {
            if place.group != expected.group {
                expected = Place {
                    group: expected.group + 1,
                    row: 0,
                };
            }
            assert_eq!(*place, expected, "rows are placed in order");
            expected.row += 1;
        }
        let footer = footer(&bytes);
        for (row, place) in rows.iter().zip(&places) {
            let range = footer.page(*place).expect("the row's page");
            let page = bytes[range.start as usize..range.end as usize].to_vec();
            for other in &rows {
                let holds = page.windows(other.sealed.len()).any(|w| w == other.sealed);
                assert_eq!(
                    holds,
                    other.version == row.version,
                    "the page of {:?} holds only its own value",
                    row.version
                );
            }
            assert_eq!(
                footer.sealed(*place, page),
                Some(row.sealed.clone()),
                "{place:?} reads back byte for byte"
            );
        }
    }

    /// A place past the file's row groups, or past its group's rows, has
    /// no page.
    #[test]
    fn a_place_the_file_does_not_hold_has_no_page() {
        let rows: Vec<Row> = (1..=4).map(row).collect();
        let (bytes, places) = packed(&rows, 100);
        let footer = footer(&bytes);
        let last = *places.last().expect("a row");
        for missing in [
            Place {
                group: last.group + 1,
                row: 0,
            },
            Place {
                group: last.group,
                row: last.row + 1,
            },
            Place {
                group: u32::MAX,
                row: u32::MAX,
            },
        ] {
            assert_eq!(footer.page(missing), None, "{missing:?}");
        }
    }

    /// Bytes that are not the page of one value decode to nothing: cut
    /// short, one byte changed in its length, a header alone, or nothing.
    #[test]
    fn bytes_that_are_not_a_page_of_one_value_decode_to_nothing() {
        let rows: Vec<Row> = (1..=3).map(row).collect();
        let (bytes, places) = packed(&rows, usize::MAX);
        let footer = footer(&bytes);
        let place = places[1];
        let range = footer.page(place).expect("a page");
        let page = bytes[range.start as usize..range.end as usize].to_vec();
        assert!(
            footer.sealed(place, page.clone()).is_some(),
            "the page itself"
        );
        let value_at = page.len() - rows[1].sealed.len();
        let mut longer = page.clone();
        longer[value_at - 4] = longer[value_at - 4].wrapping_add(1);
        for (what, bad) in [
            ("cut short", page[..page.len() - 1].to_vec()),
            ("its value's length changed", longer),
            ("its header alone", page[..value_at - 4].to_vec()),
            ("nothing", Vec::new()),
        ] {
            assert_eq!(footer.sealed(place, bad), None, "{what}");
        }
    }

    /// The end of a file that is not Parquet, or whose footer is cut, or a
    /// Parquet file not written as a pack, is not a pack file's end.
    #[test]
    fn an_end_that_is_not_a_pack_files_is_refused() {
        let rows: Vec<Row> = (1..=2).map(row).collect();
        let (bytes, _) = packed(&rows, usize::MAX);
        let size = bytes.len() as u64;
        // Parquet has no checksum over its footer, so a changed byte inside
        // it can still parse; the AES-GCM open of the value is the check
        // for that. What the format itself shows is checked here.
        let mut magic = bytes.clone();
        *magic.last_mut().expect("a byte") ^= 0xff;
        let mut long = bytes.clone();
        let at = bytes.len() - 8;
        long[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            Footer::parse(vec![0; 8], size).is_none(),
            "no PAR1 at the end"
        );
        assert!(Footer::parse(magic, size).is_none(), "PAR1 changed");
        assert!(
            Footer::parse(long, size).is_none(),
            "a footer longer than the file"
        );
        assert!(
            Footer::parse(bytes[..bytes.len() - 1].to_vec(), size - 1).is_none(),
            "a file cut short by one byte"
        );
        // A Parquet file with no sealed column, and one without the schema
        // key, are not packs.
        let other = {
            use parquet::file::properties::WriterProperties;
            use parquet::file::writer::SerializedFileWriter;
            use parquet::schema::parser::parse_message_type;
            let schema =
                Arc::new(parse_message_type("message m { required int64 n; }").expect("a schema"));
            let writer = SerializedFileWriter::new(
                Vec::new(),
                schema,
                Arc::new(WriterProperties::builder().build()),
            )
            .expect("a writer");
            writer.into_inner().expect("the file")
        };
        assert!(
            Footer::parse(other.clone(), other.len() as u64).is_none(),
            "a Parquet file that is not a pack"
        );
    }

    /// A Parquet file of `n` rows of `schema`, two values to a page, with
    /// the pack schema key if `keyed`.
    fn parquet_of(schema: &str, keyed: bool, n: u8) -> Vec<u8> {
        use parquet::column::writer::ColumnWriter;
        use parquet::data_type::{ByteArray, FixedLenByteArray};
        use parquet::file::metadata::KeyValue;
        use parquet::file::properties::WriterProperties;
        use parquet::file::writer::SerializedFileWriter;
        use parquet::schema::parser::parse_message_type;
        let schema = Arc::new(parse_message_type(schema).expect("a schema"));
        let keys = keyed.then(|| vec![KeyValue::new(SCHEMA_KEY.to_owned(), SCHEMA.to_owned())]);
        let properties = WriterProperties::builder()
            .set_dictionary_enabled(false)
            .set_data_page_row_count_limit(2)
            .set_key_value_metadata(keys)
            .build();
        let mut writer =
            SerializedFileWriter::new(Vec::new(), schema, Arc::new(properties)).expect("a writer");
        let rows: Vec<Row> = (1..=n).map(row).collect();
        let mut group = writer.next_row_group().expect("a group");
        while let Some(mut column) = group.next_column().expect("a column") {
            // One value per call: the page row limit is checked between
            // calls.
            for r in &rows {
                match column.untyped() {
                    ColumnWriter::FixedLenByteArrayColumnWriter(w) => w
                        .write_batch(&[FixedLenByteArray::from(r.version.0.to_vec())], None, None)
                        .expect("writes"),
                    ColumnWriter::Int64ColumnWriter(w) => {
                        w.write_batch(&[r.stored_at], None, None).expect("writes")
                    }
                    ColumnWriter::ByteArrayColumnWriter(w) => {
                        let optional = w.get_descriptor().max_def_level() > 0;
                        w.write_batch(
                            &[ByteArray::from(r.sealed.clone())],
                            optional.then_some(&[1][..]),
                            None,
                        )
                        .expect("writes")
                    }
                    _ => panic!("a column this test does not write"),
                };
            }
            column.close().expect("closes");
        }
        group.close().expect("closes");
        writer.into_inner().expect("the file")
    }

    /// The pack columns, `sealed` as given.
    fn columns(sealed: &str) -> String {
        format!(
            "message m {{ required fixed_len_byte_array(16) version (UUID); \
             required int64 stored_at (TIMESTAMP(NANOS, true)); {sealed} }}"
        )
    }

    /// Files that look like packs but are not written as packs are are
    /// refused: one without the pack schema key, one whose `sealed` may be
    /// NULL; and in one with two values to a page, a row that does not
    /// begin a page has no page of its own, though the offset index has an
    /// entry at its number.
    #[test]
    fn a_file_not_written_as_a_pack_is_refused() {
        let parse = |bytes: &[u8]| Footer::parse(bytes.to_vec(), bytes.len() as u64);
        let unkeyed = parquet_of(&columns("required binary sealed;"), false, 3);
        assert!(parse(&unkeyed).is_none(), "no pack schema key");
        let optional = parquet_of(&columns("optional binary sealed;"), true, 3);
        assert!(parse(&optional).is_none(), "sealed may be NULL");
        let shared = parquet_of(&columns("required binary sealed;"), true, 4);
        let Some(Parsed::Footer(footer)) = parse(&shared) else {
            panic!("the pack columns and key, read as a pack")
        };
        assert!(
            footer.page(Place { group: 0, row: 0 }).is_some(),
            "the first row begins the page"
        );
        assert_eq!(
            footer.page(Place { group: 0, row: 1 }),
            None,
            "the second row shares the first one's page; entry 1 is the page of rows 2 and 3"
        );
    }

    /// `parse_name` reads exactly the names `name` writes.
    #[test]
    fn only_a_pack_files_own_name_parses() {
        let file = FileId::generate();
        let good = name("2026-11-06", file);
        assert_eq!(
            parse_name(&good),
            Some(("2026-11-06".to_owned(), file)),
            "a name name() writes"
        );
        for bad in [
            good.replace("date=", ""),
            good.replace(".parquet", ".tmp"),
            good.replace("2026-11-06", "2026-11-6"),
            good.replace("2026-11-06", "2026-11-06-01"),
            good.replace("2026-11-06", "yyyy-mm-dd"),
            good.replace("part-", "part"),
            good.replace("parquet/", "objects/"),
            format!("{good}.x"),
            "parquet/not-ours.txt".to_owned(),
        ] {
            assert_eq!(parse_name(&bad), None, "not a pack file's name: {bad}");
        }
        assert_eq!(
            date_of(1_792_756_800_000_000_000).as_deref(),
            Some("2026-10-23"),
            "the UTC date of a store time"
        );
    }

    #[test]
    fn a_pack_file_is_named_by_its_date_and_id() {
        let file = FileId([0xab; 16]);
        assert_eq!(
            name("2026-11-06", file),
            format!("parquet/date=2026-11-06/part-{}.parquet", "ab".repeat(16)),
            "a Hive date directory, then the file id"
        );
    }
}
