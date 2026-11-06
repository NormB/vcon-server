//! One export file: the rows of one tenant and one store date, written as
//! Parquet with the column writer and no compression. The `sealed` column
//! holds each version's object byte for byte, so it opens with the version's
//! wrapped data key exactly as the object does. The tenant's metadata level
//! says which clear columns are written beside it.
//!
//! Every byte-array column is written plain, without a dictionary and
//! without statistics: a dictionary page or a min/max statistic would copy
//! a value into the file a second time, outside its row. Statistics are on
//! only for the time and count columns, whose values the columns already
//! show. Each value is written in a page of its own, so a later reader can
//! fetch one row's `sealed` value with one ranged read through the offset
//! index.

use std::sync::Arc;

use parquet::basic::Compression;
use parquet::column::writer::ColumnWriter;
use parquet::data_type::{ByteArray, FixedLenByteArray};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::file::writer::SerializedFileWriter;
use parquet::schema::parser::parse_message_type;
use parquet::schema::types::ColumnPath;
use uuid::Uuid;

use super::Metadata;
use crate::ids::VersionId;

/// The schema version written in each file's key-value metadata, under
/// [`SCHEMA_KEY`]. It changes when a column is added, removed or changed.
pub const SCHEMA: &str = "1";

/// The key-value metadata key of the schema version.
pub const SCHEMA_KEY: &str = "vcon-server.schema";

/// The key-value metadata key of the metadata level a file was written at.
pub const LEVEL_KEY: &str = "vcon-server.metadata";

/// The columns every file has.
const FIXED: &str = "
    required fixed_len_byte_array(16) version (UUID);
    required int64 stored_at (TIMESTAMP(NANOS, true));
    required binary sealed;";

/// The columns the `counts` level adds.
const COUNTS: &str = "
    optional binary form (STRING);
    optional boolean gzip;
    optional int64 size;
    optional int64 created_at (TIMESTAMP(NANOS, true));
    optional int64 updated_at (TIMESTAMP(NANOS, true));
    optional int32 party_count;
    optional int32 dialog_count;";

/// The columns the `full` level adds to `counts`.
const FULL: &str = "
    optional fixed_len_byte_array(16) uuid (UUID);
    optional binary subject (STRING);
    optional binary parties (JSON);";

/// The columns with statistics: times and counts, whose values the
/// columns show anyway.
const WITH_STATISTICS: [&str; 6] = [
    "stored_at",
    "size",
    "created_at",
    "updated_at",
    "party_count",
    "dialog_count",
];

/// The Parquet schema of a file at `level`.
pub fn schema(level: Metadata) -> String {
    let more = match level {
        Metadata::None => String::new(),
        Metadata::Counts => COUNTS.to_owned(),
        Metadata::Full => format!("{COUNTS}{FULL}"),
    };
    format!("message vcon_export {{{FIXED}{more}\n}}")
}

/// One exported version. The clear columns are read only at the level that
/// writes them; at a lower level they are left out of the file whatever
/// they hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub version: VersionId,
    /// Nanoseconds since the epoch, as the catalog keeps it.
    pub stored_at: i64,
    /// The version's object, byte for byte.
    pub sealed: Vec<u8>,
    pub about: About,
}

/// What the `counts` and `full` levels write of a version. None is NULL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct About {
    pub form: Option<String>,
    pub gzip: Option<bool>,
    pub size: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub party_count: Option<i32>,
    pub dialog_count: Option<i32>,
    pub uuid: Option<Uuid>,
    pub subject: Option<String>,
    /// The parties as a JSON array of objects with `tel`, `mailto` and
    /// `name`.
    pub parties: Option<String>,
}

/// Where a row is in its file: its row group, and its row in that group,
/// both counted from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub group: u32,
    pub row: u32,
}

/// How large a row group and a file may grow before the next is begun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    /// Bytes of `sealed` values buffered before they are written as one row
    /// group.
    pub row_group: usize,
    /// Bytes a file may hold before the next row starts a new file.
    pub file: usize,
}

/// 64 MiB row groups in files of about 128 MiB: each file is built in
/// memory and written with one call, an S3 PUT among them.
pub const SIZES: Sizes = Sizes {
    row_group: 64 << 20,
    file: 128 << 20,
};

/// Why a file could not be written.
#[derive(Debug)]
pub struct WriteError(pub parquet::errors::ParquetError);

impl From<parquet::errors::ParquetError> for WriteError {
    fn from(e: parquet::errors::ParquetError) -> WriteError {
        WriteError(e)
    }
}

/// A file being written: rows are buffered into row groups, and the file
/// is in memory until `finish`.
pub struct File {
    writer: SerializedFileWriter<Vec<u8>>,
    level: Metadata,
    pending: Vec<Row>,
    pending_bytes: usize,
    sizes: Sizes,
    rows: u64,
    /// Row groups written out.
    groups: u32,
}

impl std::fmt::Debug for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("File")
            .field("level", &self.level)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl File {
    /// A new, empty file at `level`.
    pub fn new(level: Metadata, sizes: Sizes) -> Result<File, WriteError> {
        let schema = Arc::new(parse_message_type(&schema(level))?);
        let mut properties = WriterProperties::builder()
            .set_compression(Compression::UNCOMPRESSED)
            .set_dictionary_enabled(false)
            .set_statistics_enabled(EnabledStatistics::None)
            .set_data_page_row_count_limit(1)
            .set_key_value_metadata(Some(vec![
                KeyValue::new(SCHEMA_KEY.to_owned(), SCHEMA.to_owned()),
                KeyValue::new(LEVEL_KEY.to_owned(), level.as_str().to_owned()),
            ]));
        for name in WITH_STATISTICS {
            properties = properties
                .set_column_statistics_enabled(ColumnPath::from(name), EnabledStatistics::Page);
        }
        let writer = SerializedFileWriter::new(Vec::new(), schema, Arc::new(properties.build()))?;
        Ok(File {
            writer,
            level,
            pending: Vec::new(),
            pending_bytes: 0,
            sizes,
            rows: 0,
            groups: 0,
        })
    }

    /// Adds a row, and says where it is in the file; a full row group is
    /// written out.
    pub fn push(&mut self, row: Row) -> Result<Place, WriteError> {
        let too_many = || {
            WriteError(parquet::errors::ParquetError::General(
                "too many rows for one file".to_owned(),
            ))
        };
        let place = Place {
            group: self.groups,
            row: u32::try_from(self.pending.len()).map_err(|_| too_many())?,
        };
        self.pending_bytes += row.sealed.len();
        self.pending.push(row);
        self.rows += 1;
        if self.pending_bytes >= self.sizes.row_group {
            self.flush()?;
        }
        Ok(place)
    }

    /// Writes the buffered rows as one row group.
    fn flush(&mut self) -> Result<(), WriteError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        let mut group = self.writer.next_row_group()?;
        while let Some(mut column) = group.next_column()? {
            let name = column_name(&mut column);
            write_column(&name, &rows, column.untyped())?;
            column.close()?;
        }
        group.close()?;
        self.groups += 1;
        Ok(())
    }

    /// The bytes written so far and buffered.
    pub fn size(&self) -> usize {
        self.writer.bytes_written() + self.pending_bytes
    }

    /// Whether the file has reached its size and the next row belongs in a
    /// new one.
    pub fn full(&self) -> bool {
        self.size() >= self.sizes.file
    }

    /// The rows added.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// The whole file.
    pub fn finish(mut self) -> Result<Vec<u8>, WriteError> {
        self.flush()?;
        Ok(self.writer.into_inner()?)
    }
}

/// The name of the column a writer writes.
fn column_name(column: &mut parquet::file::writer::SerializedColumnWriter<'_>) -> String {
    let descr = match column.untyped() {
        ColumnWriter::BoolColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::Int32ColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::Int64ColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::Int96ColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::FloatColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::DoubleColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::ByteArrayColumnWriter(w) => w.get_descriptor().clone(),
        ColumnWriter::FixedLenByteArrayColumnWriter(w) => w.get_descriptor().clone(),
    };
    descr.name().to_owned()
}

/// Writes column `name` of `rows`, one value per call so each value is a
/// page of its own. An optional column writes NULL as definition level 0.
fn write_column(name: &str, rows: &[Row], writer: &mut ColumnWriter<'_>) -> Result<(), WriteError> {
    fn put<T: parquet::data_type::DataType>(
        w: &mut parquet::column::writer::ColumnWriterImpl<'_, T>,
        optional: bool,
        values: impl Iterator<Item = Option<T::T>>,
    ) -> Result<(), WriteError>
    where
        T::T: Clone,
    {
        for value in values {
            match (value, optional) {
                (Some(v), false) => w.write_batch(&[v], None, None)?,
                (Some(v), true) => w.write_batch(&[v], Some(&[1]), None)?,
                (None, true) => w.write_batch(&[], Some(&[0]), None)?,
                (None, false) => {
                    return Err(WriteError(parquet::errors::ParquetError::General(
                        "a required column has no value".to_owned(),
                    )));
                }
            };
        }
        Ok(())
    }
    let fixed = |b: &[u8]| FixedLenByteArray::from(b.to_vec());
    let bytes = |b: &[u8]| ByteArray::from(b.to_vec());
    fn a(row: &Row) -> &About {
        &row.about
    }
    match (name, writer) {
        ("version", ColumnWriter::FixedLenByteArrayColumnWriter(w)) => {
            put(w, false, rows.iter().map(|r| Some(fixed(&r.version.0))))
        }
        ("stored_at", ColumnWriter::Int64ColumnWriter(w)) => {
            put(w, false, rows.iter().map(|r| Some(r.stored_at)))
        }
        ("sealed", ColumnWriter::ByteArrayColumnWriter(w)) => {
            put(w, false, rows.iter().map(|r| Some(bytes(&r.sealed))))
        }
        ("form", ColumnWriter::ByteArrayColumnWriter(w)) => put(
            w,
            true,
            rows.iter()
                .map(|r| a(r).form.as_deref().map(|t| bytes(t.as_bytes()))),
        ),
        ("gzip", ColumnWriter::BoolColumnWriter(w)) => put(w, true, rows.iter().map(|r| a(r).gzip)),
        ("size", ColumnWriter::Int64ColumnWriter(w)) => {
            put(w, true, rows.iter().map(|r| a(r).size))
        }
        ("created_at", ColumnWriter::Int64ColumnWriter(w)) => {
            put(w, true, rows.iter().map(|r| a(r).created_at))
        }
        ("updated_at", ColumnWriter::Int64ColumnWriter(w)) => {
            put(w, true, rows.iter().map(|r| a(r).updated_at))
        }
        ("party_count", ColumnWriter::Int32ColumnWriter(w)) => {
            put(w, true, rows.iter().map(|r| a(r).party_count))
        }
        ("dialog_count", ColumnWriter::Int32ColumnWriter(w)) => {
            put(w, true, rows.iter().map(|r| a(r).dialog_count))
        }
        ("uuid", ColumnWriter::FixedLenByteArrayColumnWriter(w)) => put(
            w,
            true,
            rows.iter().map(|r| a(r).uuid.map(|u| fixed(u.as_bytes()))),
        ),
        ("subject", ColumnWriter::ByteArrayColumnWriter(w)) => put(
            w,
            true,
            rows.iter()
                .map(|r| a(r).subject.as_deref().map(|t| bytes(t.as_bytes()))),
        ),
        ("parties", ColumnWriter::ByteArrayColumnWriter(w)) => put(
            w,
            true,
            rows.iter()
                .map(|r| a(r).parties.as_deref().map(|t| bytes(t.as_bytes()))),
        ),
        (other, _) => Err(WriteError(parquet::errors::ParquetError::General(format!(
            "no value for column {other}"
        )))),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::record::Field;

    use super::*;

    /// The rows of `bytes`, each as (column name, value) pairs.
    pub(crate) fn read(bytes: &[u8]) -> Vec<Vec<(String, Field)>> {
        let mut file = tempfile::tempfile().expect("a temporary file");
        std::io::Write::write_all(&mut file, bytes).expect("writes it");
        let reader = SerializedFileReader::new(file).expect("a Parquet file");
        reader
            .get_row_iter(None)
            .expect("its rows")
            .map(|row| {
                row.expect("a row")
                    .get_column_iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .collect()
    }

    /// The file's key-value metadata.
    pub(crate) fn key_values(bytes: &[u8]) -> Vec<(String, Option<String>)> {
        let mut file = tempfile::tempfile().expect("a temporary file");
        std::io::Write::write_all(&mut file, bytes).expect("writes it");
        let reader = SerializedFileReader::new(file).expect("a Parquet file");
        reader
            .metadata()
            .file_metadata()
            .key_value_metadata()
            .into_iter()
            .flatten()
            .map(|kv| (kv.key.clone(), kv.value.clone()))
            .collect()
    }

    pub(crate) fn written(level: Metadata, rows: &[Row], sizes: Sizes) -> Vec<u8> {
        let mut file = File::new(level, sizes).expect("a new file");
        for row in rows {
            file.push(row.clone()).expect("adds a row");
        }
        file.finish().expect("the file")
    }

    fn row(n: u8) -> Row {
        // Not UTF-8, as a gzip body or ciphertext often is not.
        let mut sealed = vec![0x1f, 0x8b, 0x00, 0xff];
        sealed.extend(std::iter::repeat_n(n, usize::from(n) * 7));
        Row {
            version: VersionId([n; 16]),
            stored_at: 1_762_300_000_000_000_000 + i64::from(n),
            sealed,
            about: About {
                form: Some("unsigned".to_owned()),
                gzip: Some(n.is_multiple_of(2)),
                size: Some(i64::from(n) * 100),
                created_at: (!n.is_multiple_of(3)).then_some(1_700_000_000_000_000_000),
                updated_at: None,
                party_count: Some(2),
                dialog_count: Some(i32::from(n)),
                uuid: Some(Uuid::from_bytes([0xa0 | (n & 0x0f); 16])),
                subject: Some(format!("SUBJECT-{n}")),
                parties: Some(format!(
                    r#"[{{"tel":"+1555555{:04}"}}]"#,
                    100 + u16::from(n)
                )),
            },
        }
    }

    fn names(rows: &[Vec<(String, Field)>]) -> Vec<String> {
        rows[0].iter().map(|(name, _)| name.clone()).collect()
    }

    const BIG: Sizes = SIZES;

    /// Each level writes its own columns, in order, and no more.
    #[test]
    fn each_level_writes_its_columns() {
        let rows = [row(1)];
        let fixed = ["version", "stored_at", "sealed"];
        let counts = [
            "form",
            "gzip",
            "size",
            "created_at",
            "updated_at",
            "party_count",
            "dialog_count",
        ];
        let full = ["uuid", "subject", "parties"];
        for (level, expected) in [
            (Metadata::None, fixed.to_vec()),
            (Metadata::Counts, [&fixed[..], &counts].concat()),
            (Metadata::Full, [&fixed[..], &counts, &full].concat()),
        ] {
            let bytes = written(level, &rows, BIG);
            assert_eq!(names(&read(&bytes)), expected, "{level:?}");
            let kv = key_values(&bytes);
            assert!(
                kv.contains(&(LEVEL_KEY.to_owned(), Some(level.as_str().to_owned()))),
                "{level:?}: the level is recorded: {kv:?}"
            );
            assert!(
                kv.contains(&(SCHEMA_KEY.to_owned(), Some(SCHEMA.to_owned()))),
                "{level:?}: the schema version is recorded: {kv:?}"
            );
        }
    }

    /// Every value reads back as written, NULLs included, across row
    /// groups: `sealed` byte for byte, though it is not UTF-8.
    #[test]
    fn every_value_reads_back_as_written() {
        let rows: Vec<Row> = (1..=9).map(row).collect();
        let small = Sizes {
            row_group: 40,
            file: usize::MAX,
        };
        let back = read(&written(Metadata::Full, &rows, small));
        assert_eq!(back.len(), rows.len(), "one row each");
        for (row, got) in rows.iter().zip(&back) {
            let value = |name: &str| {
                got.iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| panic!("no column {name}"))
            };
            let bytes = |name: &str| match value(name) {
                Field::Bytes(b) => b.data().to_vec(),
                Field::Str(s) => s.into_bytes(),
                other => panic!("{name}: {other:?}"),
            };
            assert_eq!(bytes("sealed"), row.sealed, "sealed, byte for byte");
            assert_eq!(
                value("stored_at"),
                Field::Long(row.stored_at),
                "store time in nanoseconds"
            );
            assert_eq!(bytes("version"), row.version.0, "the version id's 16 bytes");
            let about = &row.about;
            // The record reader has no NANOS timestamp, so it gives the INT64.
            let time = |t: Option<i64>| t.map_or(Field::Null, Field::Long);
            assert_eq!(value("created_at"), time(about.created_at), "created_at");
            assert_eq!(value("updated_at"), Field::Null, "a NULL time");
            assert_eq!(value("gzip"), Field::Bool(about.gzip.expect("set")), "gzip");
            assert_eq!(value("size"), Field::Long(about.size.expect("set")), "size");
            assert_eq!(value("party_count"), Field::Int(2), "party_count");
            assert_eq!(
                value("dialog_count"),
                Field::Int(about.dialog_count.expect("set")),
                "dialog_count"
            );
            assert_eq!(bytes("form"), b"unsigned", "form");
            assert_eq!(
                bytes("uuid"),
                about.uuid.expect("set").as_bytes(),
                "the uuid's 16 bytes"
            );
            assert_eq!(
                bytes("subject"),
                about.subject.as_deref().expect("set").as_bytes(),
                "subject"
            );
            assert_eq!(
                bytes("parties"),
                about.parties.as_deref().expect("set").as_bytes(),
                "parties"
            );
        }
    }

    /// An empty file is still a Parquet file, with no rows.
    #[test]
    fn a_file_with_no_rows_reads_as_empty() {
        assert!(
            read(&written(Metadata::None, &[], BIG)).is_empty(),
            "no row"
        );
    }

    /// The file is full once it holds its size; until then it is not.
    #[test]
    fn a_file_is_full_at_its_size() {
        let sizes = Sizes {
            row_group: 64,
            file: 300,
        };
        let mut file = File::new(Metadata::None, sizes).expect("a new file");
        assert!(!file.full(), "empty");
        let mut pushed = 0;
        while !file.full() {
            file.push(row(9)).expect("adds a row");
            pushed += 1;
            assert!(pushed < 100, "a file of {} bytes never fills", sizes.file);
        }
        assert!(file.size() >= sizes.file, "full at its size");
        assert_eq!(file.rows(), pushed, "every pushed row is counted");
        assert_eq!(
            read(&file.finish().expect("the file")).len(),
            pushed as usize,
            "every pushed row is in the file"
        );
    }

    /// The bytes of a clear value, in any column, appear in the file only
    /// where its row holds it: no dictionary page and no statistic copies
    /// it. `sealed` holds a marker that must appear exactly once.
    #[test]
    fn no_byte_array_value_is_copied_into_the_file() {
        let marker = b"SEALED-MARKER-SEALED-MARKER";
        let mut rows: Vec<Row> = (1..=4).map(row).collect();
        for row in &mut rows {
            row.sealed = marker.to_vec();
            row.about.subject = Some("SUBJECT-SAME".to_owned());
        }
        let bytes = written(Metadata::Full, &rows, BIG);
        let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
        assert_eq!(count(marker), rows.len(), "once per row, no more");
        assert_eq!(count(b"SUBJECT-SAME"), rows.len(), "once per row, no more");
        for row in &rows {
            let uuid = row.about.uuid.expect("set");
            assert_eq!(count(uuid.as_bytes()), 1, "a uuid once, in its row");
        }
    }

    /// One page per value: the offset index lists a page for each row of
    /// `sealed`, starting at that row.
    #[test]
    fn each_sealed_value_is_a_page_of_its_own() {
        use parquet::file::serialized_reader::ReadOptionsBuilder;
        let rows: Vec<Row> = (1..=5).map(row).collect();
        let bytes = written(Metadata::None, &rows, BIG);
        let mut file = tempfile::tempfile().expect("a temporary file");
        std::io::Write::write_all(&mut file, &bytes).expect("writes it");
        let options = ReadOptionsBuilder::new().with_page_index().build();
        let reader = SerializedFileReader::new_with_options(file, options).expect("reads");
        let index = reader.metadata().page_index_for_row_group(0);
        let firsts: Vec<i64> = index
            .page_locations(2)
            .expect("an offset index for sealed")
            .iter()
            .map(|p| p.first_row_index)
            .collect();
        assert_eq!(firsts, [0, 1, 2, 3, 4], "page i starts at row i");
    }
}
