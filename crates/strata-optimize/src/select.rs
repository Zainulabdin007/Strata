//! Goal-aware plan selection (does not simply pick the smallest).

use crate::bench::BenchResult;
use crate::error::{Error, Result};
use crate::goal::{OptimizationConstraints, OptimizationGoal};

/// Outcome of selecting among benched candidates.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// Index into the trials slice.
    pub index: usize,
    /// Winning bench result.
    pub result: BenchResult,
    /// Short rationale fragment used in WHY output.
    pub rationale: String,
}

/// Chooses a plan according to `goal`, among candidates that satisfy `constraints`.
pub fn select_plan(
    trials: &[BenchResult],
    goal: OptimizationGoal,
    constraints: &OptimizationConstraints,
) -> Result<Selection> {
    if trials.is_empty() {
        return Err(Error::Invalid {
            message: "no encoding candidates to select from".into(),
        });
    }

    let feasible: Vec<(usize, &BenchResult)> = trials
        .iter()
        .enumerate()
        .filter(|(_, t)| satisfies(t, constraints))
        .collect();

    let constraints_satisfied = !feasible.is_empty();
    let pool: Vec<(usize, &BenchResult)> = if constraints_satisfied {
        feasible
    } else {
        trials.iter().enumerate().collect()
    };

    let (index, rationale) = match goal {
        OptimizationGoal::MaxCompression => pick_max_compression(&pool),
        OptimizationGoal::MaxIngest => pick_max_ingest(&pool),
        OptimizationGoal::MaxDecode => pick_max_decode(&pool),
        OptimizationGoal::Balanced => pick_balanced(&pool),
    };

    let mut rationale = rationale;
    if !constraints_satisfied {
        rationale.push_str(" (no candidate fully satisfied constraints; chose best effort)");
    }

    Ok(Selection {
        index,
        result: trials[index].clone(),
        rationale,
    })
}

/// Returns true when a bench result meets all declared constraints.
pub fn satisfies(trial: &BenchResult, constraints: &OptimizationConstraints) -> bool {
    if let Some(min) = constraints.min_ingest_mb_s {
        if trial.encode_mb_s < min {
            return false;
        }
    }
    if let Some(min) = constraints.min_decode_mb_s {
        if trial.decode_mb_s < min {
            return false;
        }
    }
    if let Some(max_mem) = constraints.max_memory_bytes {
        if trial.peak_memory_bytes > max_mem {
            return false;
        }
    }
    if let Some(target) = constraints.target_compression_ratio {
        if trial.compression_ratio() + f64::EPSILON < target {
            return false;
        }
    }
    true
}

fn pick_max_compression(pool: &[(usize, &BenchResult)]) -> (usize, String) {
    let (idx, t) = pool
        .iter()
        .min_by(|a, b| {
            a.1.encoded_bytes.cmp(&b.1.encoded_bytes).then_with(|| {
                b.1.encode_mb_s
                    .partial_cmp(&a.1.encode_mb_s)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        })
        .copied()
        .expect("non-empty");
    (
        idx,
        format!(
            "sample result smallest ({} bytes) while satisfying goal/constraints",
            t.encoded_bytes
        ),
    )
}

fn pick_max_ingest(pool: &[(usize, &BenchResult)]) -> (usize, String) {
    let (idx, t) = pool
        .iter()
        .max_by(|a, b| {
            a.1.encode_mb_s
                .partial_cmp(&b.1.encode_mb_s)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.encoded_bytes.cmp(&a.1.encoded_bytes))
        })
        .copied()
        .expect("non-empty");
    (
        idx,
        format!("fastest encode on sample ({:.1} MB/s)", t.encode_mb_s),
    )
}

fn pick_max_decode(pool: &[(usize, &BenchResult)]) -> (usize, String) {
    let (idx, t) = pool
        .iter()
        .max_by(|a, b| {
            a.1.decode_mb_s
                .partial_cmp(&b.1.decode_mb_s)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.encoded_bytes.cmp(&a.1.encoded_bytes))
        })
        .copied()
        .expect("non-empty");
    (
        idx,
        format!("fastest decode on sample ({:.1} MB/s)", t.decode_mb_s),
    )
}

fn pick_balanced(pool: &[(usize, &BenchResult)]) -> (usize, String) {
    let min_bytes = pool
        .iter()
        .map(|(_, t)| t.encoded_bytes)
        .min()
        .unwrap_or(1)
        .max(1) as f64;
    let max_encode = pool
        .iter()
        .map(|(_, t)| t.encode_mb_s)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    let max_decode = pool
        .iter()
        .map(|(_, t)| t.decode_mb_s)
        .fold(0.0_f64, f64::max)
        .max(1e-9);

    let mut best_idx = pool[0].0;
    let mut best_cost = f64::INFINITY;
    for &(idx, t) in pool {
        let size_score = t.encoded_bytes as f64 / min_bytes;
        let encode_score = max_encode / t.encode_mb_s.max(1e-9);
        let decode_score = max_decode / t.decode_mb_s.max(1e-9);
        // Emphasize speed enough that a ~6% size loss with ~5× encode win picks the faster plan.
        let cost = 0.40 * size_score + 0.40 * encode_score + 0.20 * decode_score;
        if cost < best_cost {
            best_cost = cost;
            best_idx = idx;
        }
    }

    let t = pool.iter().find(|(i, _)| *i == best_idx).unwrap().1;
    (
        best_idx,
        format!(
            "balanced cost favored {:.0} bytes at {:.1} MB/s encode (not simply smallest)",
            t.encoded_bytes, t.encode_mb_s
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_codec::{EncodingPlan, EncodingStep};

    fn trial(bytes: u64, encode: f64, decode: f64) -> BenchResult {
        BenchResult {
            plan: EncodingPlan::new(vec![EncodingStep::Plain]).unwrap(),
            encoded_bytes: bytes,
            logical_bytes: 100_000_000,
            encode_mb_s: encode,
            decode_mb_s: decode,
            peak_memory_bytes: bytes * 2,
            encode_iters: 1,
            decode_iters: 1,
        }
    }

    #[test]
    fn balanced_prefers_faster_slightly_larger() {
        // Pipeline A: 48 MB @ 800 MB/s — Pipeline B: 51 MB @ 4.3 GB/s
        let a = trial(48_000_000, 800.0, 2000.0);
        let b = trial(51_000_000, 4300.0, 2500.0);
        let mut ta = a.clone();
        ta.plan = EncodingPlan::new(vec![
            EncodingStep::DeltaOfDelta,
            EncodingStep::ZigZag,
            EncodingStep::BitPacking,
        ])
        .unwrap();
        let mut tb = b.clone();
        tb.plan = EncodingPlan::new(vec![EncodingStep::ZigZag, EncodingStep::BitPacking]).unwrap();

        let sel = select_plan(
            &[ta, tb.clone()],
            OptimizationGoal::Balanced,
            &OptimizationConstraints::default(),
        )
        .unwrap();
        assert_eq!(sel.result.encoded_bytes, 51_000_000);
        assert!(sel.rationale.contains("balanced"));
    }

    #[test]
    fn max_compression_picks_smallest() {
        let trials = vec![
            trial(48_000_000, 800.0, 2000.0),
            trial(51_000_000, 4300.0, 2500.0),
        ];
        let sel = select_plan(
            &trials,
            OptimizationGoal::MaxCompression,
            &OptimizationConstraints::default(),
        )
        .unwrap();
        assert_eq!(sel.result.encoded_bytes, 48_000_000);
    }

    #[test]
    fn constraints_filter_slow_plans() {
        let trials = vec![trial(10, 100.0, 100.0), trial(20, 5000.0, 5000.0)];
        let constraints = OptimizationConstraints {
            min_ingest_mb_s: Some(1000.0),
            ..Default::default()
        };
        let sel = select_plan(&trials, OptimizationGoal::MaxCompression, &constraints).unwrap();
        assert_eq!(sel.result.encoded_bytes, 20);
    }
}
