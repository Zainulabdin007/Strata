//! Record parser interfaces and CSV / JSONL implementations.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use strata_core::{CompressionProfile, FieldValue, InputFormat, Record, RecordBatch, Schema};

use crate::error::{Error, Result};
use crate::value::{parse_json_value, parse_optional_text};

/// Options controlling parse behavior.
#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    /// Maximum records to accumulate into a single [`RecordBatch`] when using
    /// [`RecordParser::parse_file`]. `None` means the whole file.
    pub max_batch_rows: Option<usize>,
}

/// Parses a single input file into Strata records.
///
/// New formats (Parquet, binary) should implement this trait.
pub trait RecordParser: Send {
    /// Stable format name.
    fn format_name(&self) -> &'static str;

    /// Parses `path` into a [`RecordBatch`] validated against `schema`.
    fn parse_file(
        &mut self,
        path: &Path,
        schema: &Schema,
        options: &ParseOptions,
    ) -> Result<RecordBatch>;

    /// Streams `path` in batches of at most `batch_rows` records.
    ///
    /// Every record is delivered exactly once through `on_batch`. Errors from
    /// `on_batch` abort parsing and propagate (no silent drops).
    fn for_each_batch(
        &mut self,
        path: &Path,
        schema: &Schema,
        batch_rows: usize,
        on_batch: &mut dyn FnMut(RecordBatch) -> Result<()>,
    ) -> Result<()>;
}

/// Opens a parser for the profile's input format.
pub fn open_parser(profile: &CompressionProfile) -> Result<Box<dyn RecordParser>> {
    match profile.input_format {
        InputFormat::Dbn => Ok(Box::new(crate::dbn::DbnParser)),
        InputFormat::Csv => Ok(Box::new(CsvParser::new())),
        InputFormat::Jsonl => Ok(Box::new(JsonlParser::new())),
        InputFormat::Parquet => Err(Error::FormatNotImplemented {
            format: "parquet".into(),
        }),
        InputFormat::Binary => Err(Error::FormatNotImplemented {
            format: "binary".into(),
        }),
    }
}

/// CSV parser (header row required).
#[derive(Debug, Default)]
pub struct CsvParser;

impl CsvParser {
    /// Creates a CSV parser.
    pub fn new() -> Self {
        Self
    }
}

impl RecordParser for CsvParser {
    fn format_name(&self) -> &'static str {
        "csv"
    }

    fn parse_file(
        &mut self,
        path: &Path,
        schema: &Schema,
        options: &ParseOptions,
    ) -> Result<RecordBatch> {
        let file = File::open(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(false)
            .from_reader(crate::bounded::RecordGuard::new(file, true));

        let headers = reader
            .headers()
            .map_err(|e| Error::MalformedCsv {
                path: path.to_path_buf(),
                line: 1,
                message: e.to_string(),
            })?
            .clone();

        if headers.len() > schema.len()
            || headers
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != headers.len()
        {
            return Err(Error::MalformedCsv {
                path: path.to_path_buf(),
                line: 1,
                message: "CSV columns must exactly match schema (no extra or duplicate headers)"
                    .into(),
            });
        }
        for field in &schema.fields {
            if !headers.iter().any(|h| h == field.name) {
                return Err(Error::MissingColumn {
                    path: path.to_path_buf(),
                    line: 1,
                    column: field.name.clone(),
                });
            }
        }

        let mut batch = RecordBatch::empty(schema.clone());
        let limit = options.max_batch_rows.unwrap_or(usize::MAX);

        for (idx, row) in reader.records().enumerate() {
            let line = (idx as u64).saturating_add(2); // header is line 1
            let row = row.map_err(|e| Error::MalformedCsv {
                path: path.to_path_buf(),
                line,
                message: e.to_string(),
            })?;

            let mut record = Record::new();
            for field in &schema.fields {
                let pos = headers
                    .iter()
                    .position(|h| h == field.name)
                    .expect("header checked");
                let cell = row.get(pos);
                let value = parse_optional_text(
                    &field.name,
                    cell,
                    &field.logical_type,
                    field.nullable,
                    path,
                    line,
                )?;
                record.insert(field.name.clone(), value);
            }
            batch.push(record)?;
            if batch.len() >= limit {
                break;
            }
        }

        Ok(batch)
    }

    fn for_each_batch(
        &mut self,
        path: &Path,
        schema: &Schema,
        batch_rows: usize,
        on_batch: &mut dyn FnMut(RecordBatch) -> Result<()>,
    ) -> Result<()> {
        if batch_rows == 0 {
            return Err(Error::MalformedCsv {
                path: path.to_path_buf(),
                line: 0,
                message: "batch_rows must be > 0".into(),
            });
        }
        let file = File::open(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .flexible(false)
            .from_reader(crate::bounded::RecordGuard::new(file, true));

        let headers = reader
            .headers()
            .map_err(|e| Error::MalformedCsv {
                path: path.to_path_buf(),
                line: 1,
                message: e.to_string(),
            })?
            .clone();

        if headers.len() > schema.len()
            || headers
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != headers.len()
        {
            return Err(Error::MalformedCsv {
                path: path.to_path_buf(),
                line: 1,
                message: "CSV columns must exactly match schema (no extra or duplicate headers)"
                    .into(),
            });
        }
        for field in &schema.fields {
            if !headers.iter().any(|h| h == field.name) {
                return Err(Error::MissingColumn {
                    path: path.to_path_buf(),
                    line: 1,
                    column: field.name.clone(),
                });
            }
        }

        let mut batch = RecordBatch::empty(schema.clone());
        for (idx, row) in reader.records().enumerate() {
            let line = (idx as u64).saturating_add(2);
            let row = row.map_err(|e| Error::MalformedCsv {
                path: path.to_path_buf(),
                line,
                message: e.to_string(),
            })?;

            let mut record = Record::new();
            for field in &schema.fields {
                let pos = headers
                    .iter()
                    .position(|h| h == field.name)
                    .expect("header checked");
                let cell = row.get(pos);
                let value = parse_optional_text(
                    &field.name,
                    cell,
                    &field.logical_type,
                    field.nullable,
                    path,
                    line,
                )?;
                record.insert(field.name.clone(), value);
            }
            batch.push(record)?;
            if batch.len() >= batch_rows {
                let ready = std::mem::replace(&mut batch, RecordBatch::empty(schema.clone()));
                on_batch(ready)?;
            }
        }
        if !batch.is_empty() {
            on_batch(batch)?;
        }
        Ok(())
    }
}

/// JSON Lines parser (one JSON object per line).
#[derive(Debug, Default)]
pub struct JsonlParser;

impl JsonlParser {
    /// Creates a JSONL parser.
    pub fn new() -> Self {
        Self
    }
}

impl RecordParser for JsonlParser {
    fn format_name(&self) -> &'static str {
        "jsonl"
    }

    fn parse_file(
        &mut self,
        path: &Path,
        schema: &Schema,
        options: &ParseOptions,
    ) -> Result<RecordBatch> {
        let file = File::open(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = BufReader::new(crate::bounded::RecordGuard::new(file, false));
        let mut batch = RecordBatch::empty(schema.clone());
        let limit = options.max_batch_rows.unwrap_or(usize::MAX);

        for (idx, line_res) in reader.lines().enumerate() {
            let line_no = (idx as u64).saturating_add(1);
            let line = line_res.map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let value: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| Error::MalformedJsonl {
                    path: path.to_path_buf(),
                    line: line_no,
                    message: e.to_string(),
                })?;

            let obj = value.as_object().ok_or_else(|| Error::MalformedJsonl {
                path: path.to_path_buf(),
                line: line_no,
                message: "expected a JSON object per line".into(),
            })?;

            if obj.keys().any(|name| schema.field(name).is_none()) {
                return Err(Error::MalformedJsonl {
                    path: path.to_path_buf(),
                    line: line_no,
                    message: "unexpected column outside declared schema".into(),
                });
            }
            let mut record = Record::new();
            for field in &schema.fields {
                let raw = obj.get(&field.name);
                let value = match raw {
                    None => {
                        if field.nullable {
                            FieldValue::Null
                        } else {
                            return Err(Error::MissingColumn {
                                path: path.to_path_buf(),
                                line: line_no,
                                column: field.name.clone(),
                            });
                        }
                    }
                    Some(v) => parse_json_value(
                        &field.name,
                        v,
                        &field.logical_type,
                        field.nullable,
                        path,
                        line_no,
                    )?,
                };
                record.insert(field.name.clone(), value);
            }
            batch.push(record)?;
            if batch.len() >= limit {
                break;
            }
        }

        Ok(batch)
    }

    fn for_each_batch(
        &mut self,
        path: &Path,
        schema: &Schema,
        batch_rows: usize,
        on_batch: &mut dyn FnMut(RecordBatch) -> Result<()>,
    ) -> Result<()> {
        if batch_rows == 0 {
            return Err(Error::MalformedJsonl {
                path: path.to_path_buf(),
                line: 0,
                message: "batch_rows must be > 0".into(),
            });
        }
        let file = File::open(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = BufReader::new(crate::bounded::RecordGuard::new(file, false));
        let mut batch = RecordBatch::empty(schema.clone());

        for (idx, line_res) in reader.lines().enumerate() {
            let line_no = (idx as u64).saturating_add(1);
            let line = line_res.map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let value: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| Error::MalformedJsonl {
                    path: path.to_path_buf(),
                    line: line_no,
                    message: e.to_string(),
                })?;

            let obj = value.as_object().ok_or_else(|| Error::MalformedJsonl {
                path: path.to_path_buf(),
                line: line_no,
                message: "expected a JSON object per line".into(),
            })?;

            if obj.keys().any(|name| schema.field(name).is_none()) {
                return Err(Error::MalformedJsonl {
                    path: path.to_path_buf(),
                    line: line_no,
                    message: "unexpected column outside declared schema".into(),
                });
            }
            let mut record = Record::new();
            for field in &schema.fields {
                let raw = obj.get(&field.name);
                let value = match raw {
                    None => {
                        if field.nullable {
                            FieldValue::Null
                        } else {
                            return Err(Error::MissingColumn {
                                path: path.to_path_buf(),
                                line: line_no,
                                column: field.name.clone(),
                            });
                        }
                    }
                    Some(v) => parse_json_value(
                        &field.name,
                        v,
                        &field.logical_type,
                        field.nullable,
                        path,
                        line_no,
                    )?,
                };
                record.insert(field.name.clone(), value);
            }
            batch.push(record)?;
            if batch.len() >= batch_rows {
                let ready = std::mem::replace(&mut batch, RecordBatch::empty(schema.clone()));
                on_batch(ready)?;
            }
        }
        if !batch.is_empty() {
            on_batch(batch)?;
        }
        Ok(())
    }
}

/// Parses every discovered file with the appropriate parser into one batch each.
pub fn parse_files(
    files: &[PathBuf],
    profile: &CompressionProfile,
    options: &ParseOptions,
) -> Result<Vec<(PathBuf, RecordBatch)>> {
    let schema = profile.schema()?;
    let mut parser = open_parser(profile)?;
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let batch = parser.parse_file(file, &schema, options)?;
        out.push((file.clone(), batch));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::parse_profile_str;

    fn csv_profile() -> CompressionProfile {
        parse_profile_str(
            r#"
name = "t"
input_format = "csv"
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
[[columns]]
name = "symbol"
logical_type = "categorical_string"
"#,
            "t.toml",
        )
        .unwrap()
    }

    #[test]
    fn parses_valid_csv() {
        let dir = std::env::temp_dir().join(format!("strata-csv-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ok.csv");
        std::fs::write(&path, "id,symbol\n1,AAPL\n2,MSFT\n").unwrap();

        let profile = csv_profile();
        let schema = profile.schema().unwrap();
        let batch = CsvParser::new()
            .parse_file(&path, &schema, &ParseOptions::default())
            .unwrap();
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn rejects_malformed_csv() {
        let dir = std::env::temp_dir().join(format!("strata-csv-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.csv");
        // Uneven field count with flexible(false)
        std::fs::write(&path, "id,symbol\n1\n").unwrap();

        let profile = csv_profile();
        let schema = profile.schema().unwrap();
        let err = CsvParser::new()
            .parse_file(&path, &schema, &ParseOptions::default())
            .unwrap_err();
        assert!(matches!(err, Error::MalformedCsv { .. }), "{err}");
    }

    #[test]
    fn rejects_malformed_jsonl() {
        let dir = std::env::temp_dir().join(format!("strata-jsonl-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.jsonl");
        std::fs::write(&path, "{\"id\":1}\n{not-json\n").unwrap();

        let profile = parse_profile_str(
            r#"
name = "t"
input_format = "jsonl"
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
nullable = true
"#,
            "t.toml",
        )
        .unwrap();
        let schema = profile.schema().unwrap();
        let err = JsonlParser::new()
            .parse_file(&path, &schema, &ParseOptions::default())
            .unwrap_err();
        assert!(matches!(err, Error::MalformedJsonl { .. }), "{err}");
    }

    #[test]
    fn parquet_not_implemented() {
        let profile = parse_profile_str(
            r#"
name = "t"
input_format = "parquet"
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
"#,
            "t.toml",
        )
        .unwrap();
        let err = open_parser(&profile).err().expect("expected error");
        assert!(matches!(err, Error::FormatNotImplemented { .. }));
    }
}
