//! Query planning (pushdown / skip) and streaming execution.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use strata_core::{LogicalType, Record, RecordBatch, Schema};
use strata_storage::{
    project_schema, BlockRef, DatasetIndexes, DatasetReader, SegmentReadMode, SegmentReader,
};

use crate::error::{Error, Result};
use crate::plan::{Literal, Predicate, Query};
use crate::predicate::{validate_against_schema, value_matches};

/// Execution options.
#[derive(Debug, Clone)]
pub struct QueryOptions {
    /// Use on-disk indexes for block skipping when available.
    pub use_indexes: bool,
    /// Force a full segment/block scan even when indexes exist.
    pub force_full_scan: bool,
    /// How sealed segment bytes are loaded (buffered vs mmap).
    pub segment_read_mode: SegmentReadMode,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            use_indexes: true,
            force_full_scan: false,
            segment_read_mode: SegmentReadMode::Auto,
        }
    }
}

/// Explain / diagnostics for a planned query.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExplainPlan {
    /// Projection columns.
    pub select: Vec<String>,
    /// Columns that must be decoded (projection ∪ predicate fields).
    pub decode_columns: Vec<String>,
    /// Whether indexes were consulted.
    pub used_indexes: bool,
    /// Total blocks in the dataset.
    pub total_blocks: u64,
    /// Blocks scheduled for decode after skipping.
    pub selected_blocks: u64,
    /// Human-readable pushdown notes.
    pub notes: Vec<String>,
}

/// Runtime counters accumulated while streaming.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct QueryStats {
    /// Blocks examined (decoded).
    pub blocks_scanned: u64,
    /// Blocks skipped by indexes / pruning.
    pub blocks_skipped: u64,
    /// Rows examined inside decoded blocks.
    pub rows_examined: u64,
    /// Rows emitted after filters / limit.
    pub rows_returned: u64,
    /// Batches yielded.
    pub batches_returned: u64,
}

/// Streaming query results as [`RecordBatch`] values.
pub struct QueryStream {
    reader: DatasetReader,
    out_schema: Schema,
    select: Vec<String>,
    decode_columns: Vec<String>,
    predicates: Vec<Predicate>,
    remaining: Option<u64>,
    candidates: Vec<BlockRef>,
    cursor: usize,
    stats: QueryStats,
    explain: ExplainPlan,
    done: bool,
    segment_read_mode: SegmentReadMode,
    /// Cached open segment (ordinal, reader).
    open_seg: Option<(u32, SegmentReader)>,
}

impl QueryStream {
    /// Explain plan captured at start.
    pub fn explain(&self) -> &ExplainPlan {
        &self.explain
    }

    /// Live stats (also final after the stream is exhausted).
    pub fn stats(&self) -> &QueryStats {
        &self.stats
    }

    /// Output schema (projected).
    pub fn schema(&self) -> &Schema {
        &self.out_schema
    }

    /// Pull the next matching batch, or `None` when finished.
    pub fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        if self.done {
            return Ok(None);
        }
        while self.cursor < self.candidates.len() {
            if self.remaining == Some(0) {
                self.done = true;
                return Ok(None);
            }

            let bref = self.candidates[self.cursor].clone();
            self.cursor += 1;
            self.stats.blocks_scanned += 1;

            let seg_ord = bref.segment_ordinal;
            let block_id = bref.block_id;
            let decode_cols = self.decode_columns.clone();
            let (meta, columns) = {
                let seg = self.ensure_segment(seg_ord)?;
                let block_idx = seg.block_index_for_id(block_id).ok_or_else(|| {
                    Error::invalid(format!("block id {block_id} missing in segment {seg_ord}"))
                })?;
                seg.read_block_columns_projected(block_idx, &decode_cols)?
            };
            let predicate_ordinals: Vec<_> = self
                .predicates
                .iter()
                .map(|p| {
                    decode_cols
                        .iter()
                        .position(|n| n == p.field())
                        .expect("planned predicate")
                })
                .collect();
            let select_ordinals: Vec<_> = self
                .select
                .iter()
                .map(|n| {
                    decode_cols
                        .iter()
                        .position(|d| d == n)
                        .expect("planned projection")
                })
                .collect();
            let names = std::sync::Arc::new(self.select.clone());
            let mut out_records = Vec::new();
            // This ordinal indexes rows across multiple projected columns.
            #[allow(clippy::needless_range_loop)]
            for row in 0..meta.row_count as usize {
                self.stats.rows_examined += 1;
                if !self
                    .predicates
                    .iter()
                    .zip(&predicate_ordinals)
                    .all(|(p, &i)| value_matches(Some(&columns[i][row]), p))
                {
                    continue;
                }
                let projected = Record::from_ordered(
                    names.clone(),
                    select_ordinals
                        .iter()
                        .map(|&i| columns[i][row].clone())
                        .collect(),
                );
                out_records.push(projected);
                self.stats.rows_returned += 1;
                if let Some(rem) = self.remaining.as_mut() {
                    *rem = rem.saturating_sub(1);
                    if *rem == 0 {
                        break;
                    }
                }
            }

            if out_records.is_empty() {
                continue;
            }

            let out = RecordBatch {
                schema: self.out_schema.clone(),
                records: out_records,
            };
            self.stats.batches_returned += 1;
            return Ok(Some(out));
        }

        self.done = true;
        Ok(None)
    }

    /// Collect all remaining batches into memory (convenience / tests).
    pub fn collect_all(&mut self) -> Result<Vec<RecordBatch>> {
        let mut out = Vec::new();
        while let Some(batch) = self.next_batch()? {
            out.push(batch);
        }
        Ok(out)
    }

    /// Collect all matching records.
    pub fn collect_records(&mut self) -> Result<Vec<Record>> {
        let mut out = Vec::new();
        for batch in self.collect_all()? {
            out.extend(batch.records);
        }
        Ok(out)
    }

    fn ensure_segment(&mut self, ordinal: u32) -> Result<&SegmentReader> {
        let need_open = !matches!(&self.open_seg, Some((o, _)) if *o == ordinal);
        if need_open {
            let seg = self
                .reader
                .open_segment_with_mode(ordinal as usize, self.segment_read_mode)?;
            self.open_seg = Some((ordinal, seg));
        }
        Ok(&self.open_seg.as_ref().expect("opened").1)
    }
}

/// Executes `query` against `dataset` with default options.
pub fn execute(dataset: impl AsRef<Path>, query: Query) -> Result<QueryStream> {
    execute_with_options(dataset, query, QueryOptions::default())
}

/// Executes a query with explicit options.
pub fn execute_with_options(
    dataset: impl AsRef<Path>,
    query: Query,
    options: QueryOptions,
) -> Result<QueryStream> {
    let root = dataset.as_ref().to_path_buf();
    let reader = DatasetReader::open_with_mode(&root, options.segment_read_mode)?;
    let schema = reader.schema().clone();
    let select = validate_against_schema(&query.select, &query.predicates, &schema)?;
    let decode_columns = decode_column_set(&select, &query.predicates);
    let out_schema = project_schema(&schema, &select)?;

    let (candidates, mut explain) = plan_blocks(
        &root,
        &reader,
        &schema,
        &query.predicates,
        &select,
        &decode_columns,
        &options,
    )?;
    explain.notes.push(format!(
        "segment_read_mode={}",
        options.segment_read_mode.as_str()
    ));

    let total = explain.total_blocks;
    let selected = explain.selected_blocks;
    let stats = QueryStats {
        blocks_skipped: total.saturating_sub(selected),
        ..Default::default()
    };

    Ok(QueryStream {
        reader,
        out_schema,
        select,
        decode_columns,
        predicates: query.predicates,
        remaining: query.limit,
        candidates,
        cursor: 0,
        stats,
        explain,
        done: false,
        segment_read_mode: options.segment_read_mode,
        open_seg: None,
    })
}

fn decode_column_set(select: &[String], predicates: &[Predicate]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for c in select {
        set.insert(c.clone());
    }
    for p in predicates {
        set.insert(p.field().to_string());
    }
    set.into_iter().collect()
}

fn plan_blocks(
    root: &Path,
    reader: &DatasetReader,
    schema: &Schema,
    predicates: &[Predicate],
    select: &[String],
    decode_columns: &[String],
    options: &QueryOptions,
) -> Result<(Vec<BlockRef>, ExplainPlan)> {
    let all = all_blocks(reader)?;
    let total_blocks = all.len() as u64;
    let mut notes = Vec::new();
    let mut used_indexes = false;

    let candidates = if options.force_full_scan || !options.use_indexes || predicates.is_empty() {
        if options.force_full_scan {
            notes.push("forced full scan".into());
        } else if predicates.is_empty() {
            notes.push("no predicates; full scan".into());
        } else {
            notes.push("indexes disabled; full scan".into());
        }
        all
    } else {
        match DatasetIndexes::open(root) {
            Ok(indexes) => {
                used_indexes = true;
                let pruned =
                    match prune_with_indexes(&indexes, schema, predicates, &all, &mut notes) {
                        Ok(v) => v,
                        Err(e) => {
                            used_indexes = false;
                            notes.push(format!("unusable index; full scan: {e}"));
                            all.clone()
                        }
                    };
                notes.push(format!(
                    "index prune: {} / {} blocks selected",
                    pruned.len(),
                    total_blocks
                ));
                pruned
            }
            Err(_) => {
                notes.push("indexes missing; full scan (run `strata index rebuild`)".into());
                all
            }
        }
    };

    let explain = ExplainPlan {
        select: select.to_vec(),
        decode_columns: decode_columns.to_vec(),
        used_indexes,
        total_blocks,
        selected_blocks: candidates.len() as u64,
        notes,
    };
    Ok((candidates, explain))
}

fn all_blocks(reader: &DatasetReader) -> Result<Vec<BlockRef>> {
    let mut out = Vec::new();
    for seg_ord in 0..reader.segment_count() {
        let seg = reader.open_segment(seg_ord)?;
        let file = reader.manifest().segments[seg_ord].file.clone();
        for block in &seg.meta().blocks {
            out.push(BlockRef {
                segment_ordinal: seg_ord as u32,
                segment_file: file.clone(),
                block_id: block.id.0,
                row_count: block.row_count,
            });
        }
    }
    Ok(out)
}

fn prune_with_indexes(
    indexes: &DatasetIndexes,
    schema: &Schema,
    predicates: &[Predicate],
    all: &[BlockRef],
    notes: &mut Vec<String>,
) -> Result<Vec<BlockRef>> {
    let universe: HashSet<(u32, u32)> = all
        .iter()
        .map(|b| (b.segment_ordinal, b.block_id))
        .collect();
    let mut current: Option<HashSet<(u32, u32)>> = None;

    for pred in predicates {
        if let Some(set) = candidate_keys_for_predicate(indexes, schema, pred, notes)? {
            current = Some(match current {
                None => set,
                Some(prev) => prev.intersection(&set).copied().collect(),
            });
        }
    }

    let selected = match current {
        None => {
            notes.push("no indexable predicates; scanning all blocks".into());
            all.to_vec()
        }
        Some(keys) => {
            let filtered: Vec<BlockRef> = all
                .iter()
                .filter(|b| keys.contains(&(b.segment_ordinal, b.block_id)))
                .cloned()
                .collect();
            // Safety: never return blocks outside the dataset.
            debug_assert!(filtered
                .iter()
                .all(|b| universe.contains(&(b.segment_ordinal, b.block_id))));
            filtered
        }
    };
    Ok(selected)
}

fn candidate_keys_for_predicate(
    indexes: &DatasetIndexes,
    schema: &Schema,
    pred: &Predicate,
    notes: &mut Vec<String>,
) -> Result<Option<HashSet<(u32, u32)>>> {
    let field = pred.field();
    let ty = schema.field(field).map(|f| &f.logical_type);

    match pred {
        Predicate::Eq {
            value: Literal::String(v),
            ..
        } if matches!(
            ty,
            Some(LogicalType::CategoricalString(_) | LogicalType::Enum(_))
        ) =>
        {
            let blocks = indexes.lookup_categorical(field, v)?;
            notes.push(format!(
                "categorical eq `{field}` == \"{v}\" → {} blocks",
                blocks.len()
            ));
            Ok(Some(keys_of(&blocks)))
        }
        Predicate::In { values, .. }
            if matches!(
                ty,
                Some(LogicalType::CategoricalString(_) | LogicalType::Enum(_))
            ) =>
        {
            let mut set = HashSet::new();
            for lit in values {
                if let Literal::String(v) = lit {
                    for b in indexes.lookup_categorical(field, v)? {
                        set.insert((b.segment_ordinal, b.block_id));
                    }
                }
            }
            notes.push(format!("categorical IN `{field}` → {} blocks", set.len()));
            Ok(Some(set))
        }
        Predicate::Between { low, high, .. } if matches!(ty, Some(LogicalType::Timestamp(_))) => {
            let (lo, hi) = match (lit_i64(low), lit_i64(high)) {
                (Some(a), Some(b)) => (a.min(b), a.max(b)),
                _ => return Ok(None),
            };
            let blocks = indexes
                .lookup_time_range(field, lo, hi)
                .or_else(|_| indexes.prune_signed_range(field, lo, hi))?;
            notes.push(format!(
                "time between `{field}` [{lo},{hi}] → {} blocks",
                blocks.len()
            ));
            Ok(Some(keys_of(&blocks)))
        }
        Predicate::Between { low, high, .. }
            if matches!(ty, Some(LogicalType::UnsignedInteger(_))) =>
        {
            let (lo, hi) = match (lit_u64(low), lit_u64(high)) {
                (Some(a), Some(b)) => (a.min(b), a.max(b)),
                _ => return Ok(None),
            };
            let blocks = indexes.prune_unsigned_range(field, lo, hi)?;
            let set = keys_of(&blocks);
            notes.push(format!(
                "unsigned/sequence between `{field}` [{lo},{hi}] → {} blocks",
                set.len()
            ));
            Ok(Some(set))
        }
        Predicate::Between { low, high, .. }
            if matches!(
                ty,
                Some(LogicalType::SignedInteger(_) | LogicalType::FixedDecimal(_))
            ) =>
        {
            let (lo, hi) = match (lit_i64(low), lit_i64(high)) {
                (Some(a), Some(b)) => (a.min(b), a.max(b)),
                _ => return Ok(None),
            };
            let blocks = indexes.prune_signed_range(field, lo, hi)?;
            notes.push(format!(
                "numeric between `{field}` [{lo},{hi}] → {} blocks",
                blocks.len()
            ));
            Ok(Some(keys_of(&blocks)))
        }
        Predicate::Ge { value, .. } if matches!(ty, Some(LogicalType::UnsignedInteger(_))) => {
            if let Some(lo) = lit_u64(value) {
                let blocks = indexes.prune_unsigned_range(field, lo, u64::MAX)?;
                notes.push(format!(
                    "unsigned `{field}` >= {lo} → {} blocks",
                    blocks.len()
                ));
                return Ok(Some(keys_of(&blocks)));
            }
            Ok(None)
        }
        Predicate::Le { value, .. } if matches!(ty, Some(LogicalType::UnsignedInteger(_))) => {
            if let Some(hi) = lit_u64(value) {
                let blocks = indexes.prune_unsigned_range(field, 0, hi)?;
                notes.push(format!(
                    "unsigned `{field}` <= {hi} → {} blocks",
                    blocks.len()
                ));
                return Ok(Some(keys_of(&blocks)));
            }
            Ok(None)
        }
        Predicate::Ge { value, .. } if matches!(ty, Some(LogicalType::Timestamp(_))) => {
            if let Some(lo) = lit_i64(value) {
                let blocks = indexes
                    .lookup_time_range(field, lo, i64::MAX)
                    .or_else(|_| indexes.prune_signed_range(field, lo, i64::MAX))?;
                notes.push(format!("time `{field}` >= {lo} → {} blocks", blocks.len()));
                return Ok(Some(keys_of(&blocks)));
            }
            Ok(None)
        }
        Predicate::Le { value, .. } if matches!(ty, Some(LogicalType::Timestamp(_))) => {
            if let Some(hi) = lit_i64(value) {
                let blocks = indexes
                    .lookup_time_range(field, i64::MIN, hi)
                    .or_else(|_| indexes.prune_signed_range(field, i64::MIN, hi))?;
                notes.push(format!("time `{field}` <= {hi} → {} blocks", blocks.len()));
                return Ok(Some(keys_of(&blocks)));
            }
            Ok(None)
        }
        Predicate::Eq {
            value: Literal::U64(seq),
            ..
        } if matches!(ty, Some(LogicalType::UnsignedInteger(_)))
            && indexes.sequence_fields().any(|f| f == field) =>
        {
            let blocks = indexes.lookup_sequence(field, *seq)?;
            notes.push(format!(
                "sequence eq `{field}` == {seq} → {} blocks",
                blocks.len()
            ));
            Ok(Some(keys_of(&blocks)))
        }
        _ => Ok(None),
    }
}

fn keys_of(blocks: &[BlockRef]) -> HashSet<(u32, u32)> {
    blocks
        .iter()
        .map(|b| (b.segment_ordinal, b.block_id))
        .collect()
}

fn lit_u64(lit: &Literal) -> Option<u64> {
    match lit {
        Literal::U64(n) => Some(*n),
        Literal::I64(n) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

fn lit_i64(lit: &Literal) -> Option<i64> {
    match lit {
        Literal::I64(n) => Some(*n),
        Literal::U64(n) if *n <= i64::MAX as u64 => Some(*n as i64),
        _ => None,
    }
}

/// Formats explain + stats for CLI.
pub fn format_explain(explain: &ExplainPlan, stats: &QueryStats) -> String {
    let mut out = String::new();
    out.push_str("strata query plan:\n");
    out.push_str(&format!("  select: {}\n", explain.select.join(", ")));
    out.push_str(&format!(
        "  decode_columns: {}\n",
        explain.decode_columns.join(", ")
    ));
    out.push_str(&format!("  used_indexes: {}\n", explain.used_indexes));
    out.push_str(&format!(
        "  blocks: selected={} / total={}\n",
        explain.selected_blocks, explain.total_blocks
    ));
    for note in &explain.notes {
        out.push_str(&format!("  - {note}\n"));
    }
    out.push_str("strata query stats:\n");
    out.push_str(&format!("  blocks_scanned={}\n", stats.blocks_scanned));
    out.push_str(&format!("  blocks_skipped={}\n", stats.blocks_skipped));
    out.push_str(&format!("  rows_examined={}\n", stats.rows_examined));
    out.push_str(&format!("  rows_returned={}\n", stats.rows_returned));
    out.push_str(&format!("  batches_returned={}\n", stats.batches_returned));
    out
}
