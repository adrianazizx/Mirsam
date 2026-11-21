use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::score::{Counts, FieldScore, OrderScore, SampleScore};

/// The evaluation set index (`ocr-eval/out/manifest.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// Bumped whenever samples or ground truth change, so results from
    /// different versions of the set are never compared.
    pub version: u32,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sample {
    pub id: String,
    /// Path of the page image, relative to the manifest.
    pub image: String,
    /// Path of the ground-truth text, relative to the manifest.
    pub truth: String,
    pub categories: Vec<String>,
    /// Values that must be recognized exactly (numbers, dates, IDs).
    #[serde(default)]
    pub fields: Vec<String>,
}

/// Quality gate for one category. Absent limits are not gated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Target {
    pub max_cer: Option<f64>,
    pub max_wer: Option<f64>,
    pub min_field_accuracy: Option<f64>,
    pub max_order_errors: Option<usize>,
}

/// Category id → gate (`ocr-eval/targets.json`).
pub type Targets = BTreeMap<String, Target>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CategoryScore {
    pub samples: usize,
    pub cer: f64,
    pub wer: f64,
    pub cer_base: f64,
    pub field_accuracy: f64,
    pub fields_total: usize,
    pub order_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub engine: String,
    pub set_version: u32,
    pub categories: BTreeMap<String, CategoryScore>,
    pub samples: BTreeMap<String, SampleScore>,
    /// Samples the engine produced no output file for (scored as empty output).
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateFailure {
    pub category: String,
    pub metric: &'static str,
    pub actual: f64,
    pub limit: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Regression {
    pub category: String,
    pub metric: &'static str,
    pub before: f64,
    pub after: f64,
}

#[derive(Default)]
struct Accumulator {
    samples: usize,
    cer: Counts,
    wer: Counts,
    cer_base: Counts,
    fields: FieldScore,
    order: OrderScore,
}

/// Aggregates per-sample scores into per-category scores.
///
/// `scores` holds one entry per manifest sample, in manifest order. A sample
/// counts towards every category it lists, and towards the synthetic `all`.
pub fn evaluate(engine: &str, manifest: &Manifest, scores: Vec<SampleScore>, missing: Vec<String>) -> Report {
    assert_eq!(manifest.samples.len(), scores.len(), "one score per manifest sample");
    let mut totals: BTreeMap<String, Accumulator> = BTreeMap::new();
    let mut samples = BTreeMap::new();

    for (sample, score) in manifest.samples.iter().zip(scores) {
        for category in sample.categories.iter().map(String::as_str).chain(std::iter::once("all")) {
            let acc = totals.entry(category.to_string()).or_default();
            acc.samples += 1;
            acc.cer.add(score.cer);
            acc.wer.add(score.wer);
            acc.cer_base.add(score.cer_base);
            acc.fields.matched += score.fields.matched;
            acc.fields.total += score.fields.total;
            acc.order.displaced_runs += score.order.displaced_runs;
            acc.order.reversed_words += score.order.reversed_words;
        }
        samples.insert(sample.id.clone(), score);
    }

    let categories = totals
        .into_iter()
        .map(|(name, acc)| {
            let score = CategoryScore {
                samples: acc.samples,
                cer: acc.cer.rate(),
                wer: acc.wer.rate(),
                cer_base: acc.cer_base.rate(),
                field_accuracy: acc.fields.rate(),
                fields_total: acc.fields.total,
                order_errors: acc.order.total(),
            };
            (name, score)
        })
        .collect();

    Report { engine: engine.to_string(), set_version: manifest.version, categories, samples, missing }
}

impl Report {
    /// Every gate this report fails. A gated category with no samples fails too:
    /// an empty category must not pass by default.
    pub fn gate_failures(&self, targets: &Targets) -> Vec<GateFailure> {
        let mut failures = Vec::new();
        for (category, target) in targets {
            let Some(score) = self.categories.get(category) else {
                failures.push(GateFailure {
                    category: category.clone(),
                    metric: "samples",
                    actual: 0.0,
                    limit: 1.0,
                });
                continue;
            };
            let mut check = |metric, actual: f64, limit: Option<f64>, higher_is_better: bool| {
                if let Some(limit) = limit {
                    let failed = if higher_is_better { actual < limit } else { actual > limit };
                    if failed {
                        failures.push(GateFailure { category: category.clone(), metric, actual, limit });
                    }
                }
            };
            check("cer", score.cer, target.max_cer, false);
            check("wer", score.wer, target.max_wer, false);
            if score.fields_total > 0 {
                check("field_accuracy", score.field_accuracy, target.min_field_accuracy, true);
            }
            check(
                "order_errors",
                score.order_errors as f64,
                target.max_order_errors.map(|n| n as f64),
                false,
            );
        }
        failures
    }

    /// Every category metric that got worse than in `baseline` (a previous report
    /// for the same engine). Different set versions are not comparable.
    pub fn regressions(&self, baseline: &Report) -> Result<Vec<Regression>, String> {
        if baseline.set_version != self.set_version {
            return Err(format!(
                "baseline is for evaluation set v{}, this run is v{}; record a new baseline",
                baseline.set_version, self.set_version
            ));
        }
        // Rates are ratios of integers; anything beyond float noise is a real change.
        const EPSILON: f64 = 1e-9;
        let mut regressions = Vec::new();
        for (category, before) in &baseline.categories {
            let Some(after) = self.categories.get(category) else { continue };
            let mut check = |metric, before: f64, after: f64, higher_is_better: bool| {
                let worse =
                    if higher_is_better { after < before - EPSILON } else { after > before + EPSILON };
                if worse {
                    regressions.push(Regression { category: category.clone(), metric, before, after });
                }
            };
            check("cer", before.cer, after.cer, false);
            check("wer", before.wer, after.wer, false);
            check("field_accuracy", before.field_accuracy, after.field_accuracy, true);
            check("order_errors", before.order_errors as f64, after.order_errors as f64, false);
        }
        Ok(regressions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::score_sample;

    fn manifest() -> Manifest {
        let sample = |id: &str, categories: &[&str]| Sample {
            id: id.into(),
            image: format!("images/{id}.png"),
            truth: format!("truth/{id}.txt"),
            categories: categories.iter().map(|c| c.to_string()).collect(),
            fields: vec![],
        };
        Manifest { version: 1, samples: vec![sample("a", &["clean"]), sample("b", &["clean", "mixed"])] }
    }

    fn report(hyp_a: &str, hyp_b: &str) -> Report {
        let scores =
            vec![score_sample("abcdefghij", hyp_a, &[]), score_sample("klmnopqrstuvwxyzabcd", hyp_b, &[])];
        evaluate("test", &manifest(), scores, vec![])
    }

    #[test]
    fn categories_are_micro_averaged() {
        // 1 error in 10 chars and 0 errors in 20 chars: 1/30, not the mean of 10% and 0%.
        let report = report("abcdefghiX", "klmnopqrstuvwxyzabcd");
        assert!((report.categories["clean"].cer - 1.0 / 30.0).abs() < 1e-12);
        assert_eq!(report.categories["mixed"].cer, 0.0);
        assert_eq!(report.categories["all"].samples, 2);
    }

    #[test]
    fn gates_fail_on_exceeded_limits_and_on_empty_categories() {
        let report = report("abcdefghiX", "klmnopqrstuvwxyzabcd");
        let mut targets = Targets::new();
        targets.insert("clean".into(), Target { max_cer: Some(0.01), ..Target::default() });
        targets.insert(
            "mixed".into(),
            Target { max_cer: Some(0.02), max_order_errors: Some(0), ..Target::default() },
        );
        targets.insert("handwriting".into(), Target { max_cer: Some(0.5), ..Target::default() });
        let failures = report.gate_failures(&targets);
        let failed: Vec<(&str, &str)> = failures.iter().map(|f| (f.category.as_str(), f.metric)).collect();
        assert_eq!(failed, vec![("clean", "cer"), ("handwriting", "samples")]);
    }

    #[test]
    fn regressions_are_detected_against_a_baseline() {
        let baseline = report("abcdefghij", "klmnopqrstuvwxyzabcd");
        let worse = report("abcdefghiX", "klmnopqrstuvwxyzabcd");
        let regressions = worse.regressions(&baseline).expect("same set version");
        assert!(regressions.iter().any(|r| r.category == "clean" && r.metric == "cer"));
        assert!(baseline.regressions(&baseline).expect("same set version").is_empty());
        // Improvements are not regressions.
        assert!(baseline.regressions(&worse).expect("same set version").is_empty());
    }

    #[test]
    fn baselines_from_another_set_version_are_rejected() {
        let mut baseline = report("abcdefghij", "klmnopqrstuvwxyzabcd");
        baseline.set_version = 2;
        assert!(report("abcdefghij", "klmnopqrstuvwxyzabcd").regressions(&baseline).is_err());
    }
}
