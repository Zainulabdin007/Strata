//! Recursive input file discovery.

use std::path::{Path, PathBuf};

use strata_core::{CompressionProfile, InputFormat};

use crate::error::{Error, Result};
use crate::glob::path_allowed;

/// Result of discovering ingest inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredInputs {
    /// Root path supplied by the user (file or directory).
    pub root: PathBuf,
    /// Sorted list of files to ingest (deterministic order).
    pub files: Vec<PathBuf>,
}

/// Optional discovery overrides (CLI / API), merged over the profile filters.
#[derive(Debug, Clone, Default)]
pub struct DiscoverOptions {
    /// Extra include globs (relative to directory roots).
    pub include_globs: Vec<String>,
    /// Extra exclude globs.
    pub exclude_globs: Vec<String>,
}

/// Discovers input files for a profile.
pub fn discover_inputs(
    path: impl AsRef<Path>,
    profile: &CompressionProfile,
) -> Result<DiscoveredInputs> {
    discover_inputs_with(path, profile, &DiscoverOptions::default())
}

/// Discovery with extra CLI/API glob overrides.
pub fn discover_inputs_with(
    path: impl AsRef<Path>,
    profile: &CompressionProfile,
    options: &DiscoverOptions,
) -> Result<DiscoveredInputs> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(Error::PathNotFound {
            path: path.to_path_buf(),
        });
    }

    let extensions = profile.input.effective_extensions(profile.input_format);
    let expected = extensions.join(", ");
    let mut include = profile.input.include_globs.clone();
    include.extend(options.include_globs.iter().cloned());
    let mut exclude = profile.input.exclude_globs.clone();
    exclude.extend(options.exclude_globs.iter().cloned());

    if include.len() + exclude.len() > 128 || include.iter().chain(&exclude).any(|p| p.len() > 256)
    {
        return Err(Error::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "at most 128 glob filters of 256 bytes each",
            ),
        });
    }
    if path.is_file() {
        ensure_extension(path, &extensions, &expected)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !path_allowed(name, &include, &exclude) {
            return Err(Error::NoFilesMatched {
                path: path.to_path_buf(),
                extensions: format_filter_message(&expected, &include, &exclude),
            });
        }
        return Ok(DiscoveredInputs {
            root: path.to_path_buf(),
            files: vec![path.to_path_buf()],
        });
    }

    if !path.is_dir() {
        return Err(Error::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "input path is neither a file nor a directory",
            ),
        });
    }

    let mut files = Vec::new();
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
            let ty = entry.file_type().map_err(|source| Error::Io {
                path: entry.path(),
                source,
            })?;
            // Never follow directory symlinks (cycles and escapes).
            if ty.is_dir() && profile.input.recursive {
                dirs.push(entry.path());
            } else if ty.is_file() && extension_matches(&entry.path(), &extensions) {
                files.push(entry.path());
            }
        }
    }
    files.retain(|file| {
        let relative = file
            .strip_prefix(path)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        path_allowed(&relative, &include, &exclude)
    });

    files.sort();
    if files.is_empty() {
        return Err(Error::NoFilesMatched {
            path: path.to_path_buf(),
            extensions: format_filter_message(&expected, &include, &exclude),
        });
    }

    Ok(DiscoveredInputs {
        root: path.to_path_buf(),
        files,
    })
}

fn format_filter_message(extensions: &str, include: &[String], exclude: &[String]) -> String {
    format!("extensions=[{extensions}]; include={include:?}; exclude={exclude:?}")
}

fn ensure_extension(path: &Path, extensions: &[String], expected: &str) -> Result<()> {
    let found = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "<none>".to_string());
    if !extension_matches(path, extensions) {
        return Err(Error::UnsupportedExtension {
            path: path.to_path_buf(),
            found,
            expected: expected.to_string(),
        });
    }
    Ok(())
}

fn extension_matches(path: &Path, extensions: &[String]) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            extensions
                .iter()
                .any(|ext| name.ends_with(&format!(".{ext}")))
        })
}

/// Returns true when the format is implemented by this crate.
pub fn format_supported(format: InputFormat) -> bool {
    format.is_supported()
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::parse_profile_str;

    fn profile_csv() -> CompressionProfile {
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
"#,
            "t.toml",
        )
        .unwrap()
    }

    #[test]
    fn discovers_nested_csv_files() {
        let root = tempfile_dir("discover-nested");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("root.csv"), "id\n1\n").unwrap();
        std::fs::write(root.join("a/b/nested.csv"), "id\n2\n").unwrap();
        std::fs::write(root.join("a/b/skip.txt"), "nope\n").unwrap();

        let found = discover_inputs(&root, &profile_csv()).unwrap();
        assert_eq!(found.files.len(), 2);
        assert!(found.files.iter().all(|p| p.extension().unwrap() == "csv"));
    }

    #[test]
    fn rejects_unsupported_extension_for_file() {
        let root = tempfile_dir("discover-ext");
        let file = root.join("data.parquet");
        std::fs::write(&file, "x").unwrap();
        let err = discover_inputs(&file, &profile_csv()).unwrap_err();
        assert!(matches!(err, Error::UnsupportedExtension { .. }));
    }

    #[test]
    fn include_glob_filters() {
        let root = tempfile_dir("discover-glob");
        std::fs::create_dir_all(root.join("keep")).unwrap();
        std::fs::create_dir_all(root.join("drop")).unwrap();
        std::fs::write(root.join("keep/a.csv"), "id\n1\n").unwrap();
        std::fs::write(root.join("drop/b.csv"), "id\n2\n").unwrap();
        let found = discover_inputs_with(
            &root,
            &profile_csv(),
            &DiscoverOptions {
                include_globs: vec!["keep/**".into()],
                exclude_globs: vec![],
            },
        )
        .unwrap();
        assert_eq!(found.files.len(), 1);
        assert!(found.files[0].ends_with("keep/a.csv") || found.files[0].ends_with("keep\\a.csv"));
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("strata-input-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
