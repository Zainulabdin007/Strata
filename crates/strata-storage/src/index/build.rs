//! Rebuild all dataset indexes from segment data.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use strata_core::{FieldValue, LogicalType, Schema, TimestampUnit};

use super::catalog::{IndexArtifact, IndexCatalog, INDEX_FORMAT_VERSION};
use super::types::{
    BlockMinMaxIndex, BlockRef, BlockStats, CategoricalIndex, CategoricalPosting, FieldMinMax,
    IndexKind, IndexValue, LatestVersionEntry, LatestVersionIndex, SequenceEntry, SequenceIndex,
    TimeEntry, TimeIndex,
};
use crate::dataset::DatasetReader;
use crate::error::{Error, Result};
use crate::path::DatasetLayout;
use crate::versioning::{is_tombstone, RecordKey};

/// Summary of an index rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRebuildReport {
    /// Dataset root.
    pub root: PathBuf,
    /// Blocks scanned.
    pub block_count: u64,
    /// Records observed.
    pub record_count: u64,
    /// Sequence indexes written.
    pub sequence_fields: Vec<String>,
    /// Time indexes written.
    pub time_fields: Vec<String>,
    /// Categorical indexes written.
    pub categorical_fields: Vec<String>,
    /// Whether block min/max stats were written.
    pub block_minmax_written: bool,
    /// Whether a latest-version index was written.
    pub latest_version_written: bool,
}

impl IndexRebuildReport {
    /// CLI summary.
    pub fn to_cli_summary(&self) -> String {
        let mut out = format!(
            "strata index rebuild: {}\n  blocks={}\n  records={}\n  block_minmax={}\n  latest_version={}\n",
            self.root.display(),
            self.block_count,
            self.record_count,
            self.block_minmax_written,
            self.latest_version_written
        );
        out.push_str(&format!(
            "  sequence: {}\n",
            join_fields(&self.sequence_fields)
        ));
        out.push_str(&format!("  time: {}\n", join_fields(&self.time_fields)));
        out.push_str(&format!(
            "  categorical: {}\n",
            join_fields(&self.categorical_fields)
        ));
        out
    }
}

fn join_fields(fields: &[String]) -> String {
    if fields.is_empty() {
        "(none)".into()
    } else {
        fields.join(", ")
    }
}

/// Rebuilds all indexes under `<dataset>/indexes/` (atomic replace).
///
/// Existing indexes are replaced. Incomplete rebuilds never leave a half-written
/// catalog: artifacts are staged under `indexes.building/` then renamed.
pub fn rebuild_indexes(root: impl AsRef<Path>) -> Result<IndexRebuildReport> {
    let layout = DatasetLayout::new(root.as_ref());
    let reader = DatasetReader::open(&layout.root)?;
    let schema = reader.schema().clone();

    let staging = layout.indexes_building_dir();
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|source| Error::Io {
            path: staging.clone(),
            source,
        })?;
    }
    fs::create_dir_all(&staging).map_err(|source| Error::Io {
        path: staging.clone(),
        source,
    })?;
    fs::create_dir_all(staging.join("sequence")).map_err(|source| Error::Io {
        path: staging.join("sequence"),
        source,
    })?;
    fs::create_dir_all(staging.join("time")).map_err(|source| Error::Io {
        path: staging.join("time"),
        source,
    })?;
    fs::create_dir_all(staging.join("categorical")).map_err(|source| Error::Io {
        path: staging.join("categorical"),
        source,
    })?;

    let plan = IndexPlan::from_schema(&schema);
    let mut built = BuiltIndexes::new(&plan);
    let key_fields = reader.manifest().key_fields.clone();
    let mut latest_map: BTreeMap<RecordKey, LatestVersionEntry> = BTreeMap::new();

    let mut block_count = 0u64;
    let mut record_count = 0u64;

    for seg_ord in 0..reader.segment_count() {
        let seg = reader.open_segment(seg_ord)?;
        let file_name = reader.manifest().segments[seg_ord].file.clone();
        let meta = seg.meta();
        for (block_idx, block_meta) in meta.blocks.iter().enumerate() {
            let batch = seg.read_block(block_idx)?;
            let block_ref = BlockRef {
                segment_ordinal: seg_ord as u32,
                segment_file: file_name.clone(),
                block_id: block_meta.id.0,
                row_count: block_meta.row_count,
            };
            built.ingest_block(&plan, &block_ref, &batch.records);
            if !key_fields.is_empty() {
                for (row_in_block, rec) in batch.records.iter().enumerate() {
                    let key = RecordKey::from_record(rec, &key_fields)?;
                    if is_tombstone(rec) {
                        latest_map.remove(&key);
                    } else {
                        latest_map.insert(
                            key.clone(),
                            LatestVersionEntry {
                                key: key.display_parts(),
                                segment_ordinal: seg_ord as u32,
                                block_id: block_meta.id.0,
                                row_in_block: row_in_block as u32,
                            },
                        );
                    }
                }
            }
            block_count += 1;
            record_count += u64::from(block_meta.row_count);
        }
    }

    let mut artifacts = Vec::new();

    // Block min/max (always).
    let minmax = BlockMinMaxIndex {
        version: INDEX_FORMAT_VERSION,
        blocks: built.block_stats,
    };
    write_json(&staging.join("block_minmax.json"), &minmax)?;
    artifacts.push(IndexArtifact {
        kind: IndexKind::BlockMinMax,
        field: String::new(),
        path: "block_minmax.json".into(),
    });

    let mut sequence_fields = Vec::new();
    for (field, entries) in built.sequence {
        let idx = SequenceIndex {
            version: INDEX_FORMAT_VERSION,
            field: field.clone(),
            entries,
        };
        let rel = format!("sequence/{field}.json");
        write_json(&staging.join(&rel), &idx)?;
        artifacts.push(IndexArtifact {
            kind: IndexKind::Sequence,
            field: field.clone(),
            path: rel,
        });
        sequence_fields.push(field);
    }

    let mut time_fields = Vec::new();
    for (field, (unit, entries)) in built.time {
        let idx = TimeIndex {
            version: INDEX_FORMAT_VERSION,
            field: field.clone(),
            unit,
            entries,
        };
        let rel = format!("time/{field}.json");
        write_json(&staging.join(&rel), &idx)?;
        artifacts.push(IndexArtifact {
            kind: IndexKind::Time,
            field: field.clone(),
            path: rel,
        });
        time_fields.push(field);
    }

    let mut categorical_fields = Vec::new();
    for (field, (is_enum, postings_map)) in built.categorical {
        let mut postings: Vec<CategoricalPosting> = postings_map
            .into_iter()
            .map(|(value, blocks)| CategoricalPosting { value, blocks })
            .collect();
        postings.sort_by(|a, b| a.value.cmp(&b.value));
        let idx = CategoricalIndex {
            version: INDEX_FORMAT_VERSION,
            field: field.clone(),
            is_enum,
            postings,
        };
        let rel = format!("categorical/{field}.json");
        write_json(&staging.join(&rel), &idx)?;
        artifacts.push(IndexArtifact {
            kind: IndexKind::Categorical,
            field: field.clone(),
            path: rel,
        });
        categorical_fields.push(field);
    }

    sequence_fields.sort();
    time_fields.sort();
    categorical_fields.sort();

    let mut latest_version_written = false;
    if !key_fields.is_empty() {
        let mut entries: Vec<LatestVersionEntry> = latest_map.into_values().collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let idx = LatestVersionIndex {
            version: INDEX_FORMAT_VERSION,
            key_fields: key_fields.clone(),
            entries,
        };
        write_json(&staging.join("latest_version.json"), &idx)?;
        artifacts.push(IndexArtifact {
            kind: IndexKind::LatestVersion,
            field: String::new(),
            path: "latest_version.json".into(),
        });
        latest_version_written = true;
    }

    let mut checksums = std::collections::BTreeMap::new();
    for artifact in &artifacts {
        checksums.insert(
            artifact.path.clone(),
            strata_format::crc32(&crate::io::read_file(&staging.join(&artifact.path))?),
        );
    }
    let catalog = IndexCatalog {
        dataset_crc: Some(strata_format::crc32(&crate::io::read_file(
            &layout.manifest_path(),
        )?)),
        checksums,
        version: INDEX_FORMAT_VERSION,
        schema_id: schema.id.as_str().to_string(),
        record_count,
        block_count,
        artifacts,
    };
    write_json(&staging.join(IndexCatalog::FILE_NAME), &catalog)?;

    // Atomic-ish replace: remove old indexes/, rename staging.
    let final_dir = layout.indexes_dir();
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|source| Error::Io {
            path: final_dir.clone(),
            source,
        })?;
    }
    fs::rename(&staging, &final_dir).map_err(|source| Error::Io {
        path: final_dir.clone(),
        source,
    })?;

    Ok(IndexRebuildReport {
        root: layout.root,
        block_count,
        record_count,
        sequence_fields,
        time_fields,
        categorical_fields,
        block_minmax_written: true,
        latest_version_written,
    })
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|e| Error::InvalidState {
        message: format!("failed to serialize index {}: {e}", path.display()),
    })?;
    fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

struct IndexPlan {
    sequence: Vec<String>,
    time: Vec<(String, String)>,      // field, unit label
    categorical: Vec<(String, bool)>, // field, is_enum
    minmax: Vec<String>,
}

impl IndexPlan {
    fn from_schema(schema: &Schema) -> Self {
        let mut sequence = Vec::new();
        let mut time = Vec::new();
        let mut categorical = Vec::new();
        let mut minmax = Vec::new();

        for field in &schema.fields {
            match &field.logical_type {
                LogicalType::UnsignedInteger(_) => {
                    sequence.push(field.name.clone());
                    minmax.push(field.name.clone());
                }
                LogicalType::SignedInteger(_) | LogicalType::FixedDecimal(_) => {
                    minmax.push(field.name.clone());
                }
                LogicalType::Timestamp(ts) => {
                    time.push((field.name.clone(), unit_label(ts.unit)));
                    minmax.push(field.name.clone());
                }
                LogicalType::CategoricalString(_) => {
                    categorical.push((field.name.clone(), false));
                    minmax.push(field.name.clone());
                }
                LogicalType::Enum(_) => {
                    categorical.push((field.name.clone(), true));
                    minmax.push(field.name.clone());
                }
                LogicalType::Boolean(_) | LogicalType::Binary(_) => {}
            }
        }

        Self {
            sequence,
            time,
            categorical,
            minmax,
        }
    }
}

fn unit_label(unit: TimestampUnit) -> String {
    match unit {
        TimestampUnit::Seconds => "seconds".into(),
        TimestampUnit::Milliseconds => "milliseconds".into(),
        TimestampUnit::Microseconds => "microseconds".into(),
        TimestampUnit::Nanoseconds => "nanoseconds".into(),
    }
}

struct BuiltIndexes {
    block_stats: Vec<BlockStats>,
    sequence: BTreeMap<String, Vec<SequenceEntry>>,
    time: BTreeMap<String, (String, Vec<TimeEntry>)>,
    categorical: BTreeMap<String, (bool, BTreeMap<String, Vec<BlockRef>>)>,
    minmax_fields: BTreeSet<String>,
}

impl BuiltIndexes {
    fn new(plan: &IndexPlan) -> Self {
        let mut sequence = BTreeMap::new();
        for f in &plan.sequence {
            sequence.insert(f.clone(), Vec::new());
        }
        let mut time = BTreeMap::new();
        for (f, unit) in &plan.time {
            time.insert(f.clone(), (unit.clone(), Vec::new()));
        }
        let mut categorical = BTreeMap::new();
        for (f, is_enum) in &plan.categorical {
            categorical.insert(f.clone(), (*is_enum, BTreeMap::new()));
        }
        let minmax_fields: BTreeSet<String> = plan.minmax.iter().cloned().collect();
        Self {
            block_stats: Vec::new(),
            sequence,
            time,
            categorical,
            minmax_fields,
        }
    }

    fn ingest_block(
        &mut self,
        plan: &IndexPlan,
        block: &BlockRef,
        records: &[strata_core::Record],
    ) {
        let mut field_acc: BTreeMap<String, FieldAccumulator> = BTreeMap::new();
        for name in &self.minmax_fields {
            field_acc.insert(name.clone(), FieldAccumulator::default());
        }
        // Also accumulate categorical distincts even if somehow missing from minmax.
        for (name, _) in &plan.categorical {
            field_acc.entry(name.clone()).or_default();
        }

        for rec in records {
            for (name, acc) in field_acc.iter_mut() {
                match rec.get(name) {
                    None | Some(FieldValue::Null) => {
                        acc.null_count += 1;
                    }
                    Some(v) => {
                        if let Some(iv) = field_value_to_index(v) {
                            acc.observe(iv);
                        }
                        if let Some(s) = categorical_string(v) {
                            acc.categories.insert(s);
                        }
                    }
                }
            }
        }

        let mut fields = Vec::new();
        for name in &plan.minmax {
            if let Some(acc) = field_acc.get(name) {
                fields.push(FieldMinMax {
                    field: name.clone(),
                    min: acc.min.clone(),
                    max: acc.max.clone(),
                    null_count: acc.null_count,
                });
            }
        }
        self.block_stats.push(BlockStats {
            block: block.clone(),
            fields,
        });

        for name in &plan.sequence {
            if let Some(acc) = field_acc.get(name) {
                if let (Some(IndexValue::U64(min)), Some(IndexValue::U64(max))) =
                    (acc.min.clone(), acc.max.clone())
                {
                    self.sequence
                        .get_mut(name)
                        .expect("sequence field")
                        .push(SequenceEntry {
                            min,
                            max,
                            block: block.clone(),
                        });
                }
            }
        }

        for (name, _) in &plan.time {
            if let Some(acc) = field_acc.get(name) {
                if let (Some(IndexValue::I64(min)), Some(IndexValue::I64(max))) =
                    (acc.min.clone(), acc.max.clone())
                {
                    self.time
                        .get_mut(name)
                        .expect("time field")
                        .1
                        .push(TimeEntry {
                            min,
                            max,
                            block: block.clone(),
                        });
                }
            }
        }

        for (name, _) in &plan.categorical {
            if let Some(acc) = field_acc.get(name) {
                let entry = self.categorical.get_mut(name).expect("categorical field");
                for value in &acc.categories {
                    entry
                        .1
                        .entry(value.clone())
                        .or_default()
                        .push(block.clone());
                }
            }
        }
    }
}

#[derive(Default)]
struct FieldAccumulator {
    min: Option<IndexValue>,
    max: Option<IndexValue>,
    null_count: u32,
    categories: BTreeSet<String>,
}

impl FieldAccumulator {
    fn observe(&mut self, value: IndexValue) {
        match (&self.min, &value) {
            (None, _) => self.min = Some(value.clone()),
            (Some(cur), _) if value < *cur => self.min = Some(value.clone()),
            _ => {}
        }
        match (&self.max, &value) {
            (None, _) => self.max = Some(value.clone()),
            (Some(cur), _) if value > *cur => self.max = Some(value.clone()),
            _ => {}
        }
    }
}

fn field_value_to_index(v: &FieldValue) -> Option<IndexValue> {
    match v {
        FieldValue::UnsignedInteger(n) => Some(IndexValue::U64(*n)),
        FieldValue::SignedInteger(n) | FieldValue::Timestamp(n) | FieldValue::FixedDecimal(n) => {
            Some(IndexValue::I64(*n))
        }
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => {
            Some(IndexValue::String(s.clone()))
        }
        FieldValue::Boolean(_) | FieldValue::Binary(_) | FieldValue::Null => None,
    }
}

fn categorical_string(v: &FieldValue) -> Option<String> {
    match v {
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => Some(s.clone()),
        _ => None,
    }
}
