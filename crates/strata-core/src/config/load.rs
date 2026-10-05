//! TOML loading helpers for compression profiles.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

use super::profile::CompressionProfile;

/// Loads and validates a [`CompressionProfile`] from a TOML file.
pub fn load_profile(path: impl AsRef<Path>) -> Result<CompressionProfile> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| Error::ProfileIo {
        path: path.to_path_buf(),
        source,
    })?;
    parse_profile_str(&text, path)
}

/// Parses and validates a profile from TOML text.
///
/// `path` is used only for error messages.
pub fn parse_profile_str(text: &str, path: impl Into<PathBuf>) -> Result<CompressionProfile> {
    let path = path.into();
    let profile: CompressionProfile =
        toml::from_str(text).map_err(|source| Error::ProfileParse {
            path: path.clone(),
            source,
        })?;
    match profile.validate() {
        Ok(()) => Ok(profile),
        Err(Error::ProfileInvalid { message, .. }) => Err(Error::ProfileInvalid { path, message }),
        Err(Error::InvalidColumn { column, message }) => Err(Error::ProfileInvalid {
            path,
            message: format!("column `{column}`: {message}"),
        }),
        Err(Error::InvalidFixedDecimal {
            field,
            precision,
            scale,
        }) => Err(Error::ProfileInvalid {
            path,
            message: format!(
                "fixed decimal field `{field}` has scale {scale} greater than precision {precision}"
            ),
        }),
        Err(Error::EmptyEnum { field }) => Err(Error::ProfileInvalid {
            path,
            message: format!("enum field `{field}` must declare at least one variant"),
        }),
        Err(Error::EmptySchema) => Err(Error::ProfileInvalid {
            path,
            message: "profile must define at least one column under `[[columns]]`".into(),
        }),
        Err(Error::DuplicateField { name }) => Err(Error::ProfileInvalid {
            path,
            message: format!("duplicate column name `{name}`"),
        }),
        Err(other) => Err(Error::ProfileInvalid {
            path,
            message: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::hints::SemanticHint;
    use crate::config::profile::InputFormat;

    const VALID: &str = r#"
name = "demo"
input_format = "csv"

[schema]
id = "demo_v1"

[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
hints = ["monotonic"]

[[columns]]
name = "symbol"
logical_type = "categorical_string"
hints = ["low_cardinality"]
encoding = ["dictionary", "bitpack"]

[block]
row_count = 1024

[segment]
target_size_bytes = 1048576

[compression]
enable_general = false
"#;

    #[test]
    fn parses_valid_profile() {
        let profile = parse_profile_str(VALID, "demo.toml").unwrap();
        assert_eq!(profile.name, "demo");
        assert_eq!(profile.input_format, InputFormat::Csv);
        assert_eq!(profile.columns.len(), 2);
        assert!(profile.columns[0].has_hint(SemanticHint::Monotonic));
        let schema = profile.schema().unwrap();
        assert_eq!(schema.len(), 2);
    }

    #[test]
    fn rejects_unknown_logical_type() {
        let text = r#"
name = "bad"
input_format = "csv"
[schema]
id = "x"
[[columns]]
name = "a"
logical_type = "not_a_type"
"#;
        let err = parse_profile_str(text, "bad.toml").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown logical type"), "{msg}");
    }

    #[test]
    fn rejects_missing_columns() {
        let text = r#"
name = "bad"
input_format = "jsonl"
columns = []

[schema]
id = "x"
"#;
        let err = parse_profile_str(text, "empty.toml").unwrap_err();
        assert!(err.to_string().contains("at least one column"), "{err}");
    }

    #[test]
    fn parses_pipeline_encoding() {
        let text = r#"
name = "pipes"
input_format = "csv"
[schema]
id = "p"
[[columns]]
name = "ts"
logical_type = "timestamp"
unit = "nanoseconds"
encoding = ["delta_of_delta", "zigzag", "bitpack"]
[[columns]]
name = "qty"
logical_type = "unsigned_integer"
width = "bits32"
encoding = ["varint"]
"#;
        let profile = parse_profile_str(text, "pipes.toml").unwrap();
        assert!(matches!(
            &profile.columns[0].encoding,
            crate::config::ColumnEncodingSpec::Pipeline(steps) if steps.len() == 3
        ));
        assert!(matches!(
            &profile.columns[1].encoding,
            crate::config::ColumnEncodingSpec::Pipeline(steps) if steps.len() == 1
        ));
    }

    #[test]
    fn rejects_empty_pipeline() {
        let text = r#"
name = "bad"
input_format = "csv"
[schema]
id = "x"
[[columns]]
name = "a"
logical_type = "boolean"
encoding = []
"#;
        let err = parse_profile_str(text, "empty-pipe.toml").unwrap_err();
        assert!(err.to_string().contains("must not be empty"), "{err}");
    }
}
