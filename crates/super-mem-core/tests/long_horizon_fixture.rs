//! Production-engine benchmark runner for the checked-in long-horizon fixture.

#![allow(clippy::too_many_lines)]

use std::{collections::BTreeMap, env, fs, path::Path};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use super_mem_core::{
    Applicability, EngineOptions, MemoryEngine, MemoryId, RecallRequest, RememberRequest,
    RetrievalSignal, classify_applicability,
};
use uuid::Uuid;

const CASE_SCHEMA: &str = "supermem.retrieval.case.v1";
const QREL_SCHEMA: &str = "supermem.retrieval.qrels.v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureCase {
    schema_version: String,
    case_id: String,
    operations: Vec<FixtureOperation>,
    recall: RecallRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureOperation {
    op: String,
    record_id: String,
    request: RememberRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureQrels {
    schema_version: String,
    case_id: String,
    relevance: BTreeMap<String, u8>,
    #[serde(default)]
    forbidden: Vec<String>,
    #[serde(default)]
    rank_before: Vec<[String; 2]>,
    #[serde(default)]
    expected_signals: BTreeMap<String, Vec<RetrievalSignal>>,
    #[serde(default)]
    expected_applicability: BTreeMap<String, Applicability>,
    #[serde(default)]
    expected_excluded: BTreeMap<String, Applicability>,
    #[serde(default)]
    expected_revision: BTreeMap<String, u32>,
    #[serde(default)]
    forbidden_body_substrings: Vec<String>,
}

#[derive(Serialize)]
struct HitReport {
    memory_id: String,
    score: f64,
    relevance: u8,
    signals: Vec<RetrievalSignal>,
    applicability: Applicability,
    revision: u32,
}

#[derive(Serialize)]
struct CaseReport {
    case_id: String,
    ordered_hits: Vec<HitReport>,
    token_budget: usize,
    estimated_tokens: usize,
    rendered_bytes: usize,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct BenchmarkReport {
    schema_version: &'static str,
    fixture_digest: String,
    commit: Option<String>,
    metrics: MetricReport,
    cases: Vec<CaseReport>,
}

fn benchmark_digest(cases: &str, qrels: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"super-mem:long-horizon-fixture:v1\0");
    hasher.update(&(cases.len() as u64).to_le_bytes());
    hasher.update(cases.as_bytes());
    hasher.update(&(qrels.len() as u64).to_le_bytes());
    hasher.update(qrels.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[derive(Serialize)]
struct MetricReport {
    cases: u32,
    mrr_at_10: f64,
    recall_at_10: f64,
    ndcg_at_10: f64,
}

#[derive(Default)]
struct DiagnosticMetrics {
    cases: u32,
    reciprocal_rank_at_10: f64,
    recall_at_10: f64,
    ndcg_at_10: f64,
}

impl DiagnosticMetrics {
    fn record(&mut self, ordered_ids: &[String], qrels: &FixtureQrels) {
        let first_essential = ordered_ids.iter().take(10).position(|id| {
            qrels
                .relevance
                .get(id)
                .is_some_and(|relevance| *relevance == 3)
        });
        self.reciprocal_rank_at_10 +=
            first_essential.map_or(0.0, |index| 1.0 / usize_as_f64(index.saturating_add(1)));

        let relevant = qrels
            .relevance
            .values()
            .filter(|relevance| **relevance >= 2)
            .count();
        let recalled = ordered_ids
            .iter()
            .take(10)
            .filter(|id| {
                qrels
                    .relevance
                    .get(*id)
                    .is_some_and(|relevance| *relevance >= 2)
            })
            .count();
        self.recall_at_10 += if relevant == 0 {
            0.0
        } else {
            usize_as_f64(recalled) / usize_as_f64(relevant)
        };

        let dcg = ordered_ids
            .iter()
            .take(10)
            .enumerate()
            .map(|(index, id)| discounted_gain(*qrels.relevance.get(id).unwrap_or(&0), index))
            .sum::<f64>();
        let mut ideal = qrels.relevance.values().copied().collect::<Vec<_>>();
        ideal.sort_unstable_by(|left, right| right.cmp(left));
        let ideal_dcg = ideal
            .into_iter()
            .take(10)
            .enumerate()
            .map(|(index, relevance)| discounted_gain(relevance, index))
            .sum::<f64>();
        self.ndcg_at_10 += if ideal_dcg <= f64::EPSILON {
            0.0
        } else {
            dcg / ideal_dcg
        };
        self.cases = self.cases.saturating_add(1);
    }

    fn summary(&self) -> MetricReport {
        let cases = f64::from(self.cases.max(1));
        MetricReport {
            cases: self.cases,
            mrr_at_10: self.reciprocal_rank_at_10 / cases,
            recall_at_10: self.recall_at_10 / cases,
            ndcg_at_10: self.ndcg_at_10 / cases,
        }
    }

    fn report(&self) {
        let summary = self.summary();
        eprintln!(
            "RETRIEVAL_FIXTURE cases={} mrr_at_10={:.6} recall_at_10={:.6} ndcg_at_10={:.6}",
            summary.cases,
            summary.mrr_at_10,
            summary.recall_at_10,
            summary.ndcg_at_10,
        );
    }
}

fn usize_as_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).expect("fixture counts fit in u32"))
}

fn discounted_gain(relevance: u8, zero_based_rank: usize) -> f64 {
    let gain = 2_f64.powi(i32::from(relevance)) - 1.0;
    gain / usize_as_f64(zero_based_rank.saturating_add(2)).log2()
}

fn fixture_text(name: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/long-horizon")
        .join(name);
    match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // The workspace-level fixtures are intentionally not part of the
            // published core crate. Workspace CI always takes the Some path.
            eprintln!(
                "skipping workspace retrieval fixture outside a repository checkout: {}",
                path.display()
            );
            None
        }
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

fn parse_jsonl<T: DeserializeOwned>(name: &str, text: &str) -> Vec<T> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("{name}:{}: {error}", index + 1))
        })
        .collect()
}

fn memory_id(value: &str) -> MemoryId {
    MemoryId(Uuid::parse_str(value).unwrap_or_else(|error| panic!("invalid UUID {value}: {error}")))
}

#[test]
fn production_engine_satisfies_long_horizon_fixture_contract() {
    let Some(case_text) = fixture_text("v1.jsonl") else {
        return;
    };
    let qrel_text = fixture_text("qrels-v1.jsonl")
        .expect("qrels must exist whenever the retrieval cases exist");
    let cases = parse_jsonl::<FixtureCase>("v1.jsonl", &case_text);
    let qrels = parse_jsonl::<FixtureQrels>("qrels-v1.jsonl", &qrel_text);
    assert!(cases.len() >= 32, "long-horizon fixture must retain at least 32 cases");
    let fixture_digest = benchmark_digest(&case_text, &qrel_text);
    let mut case_reports = Vec::new();
    assert_eq!(cases.len(), qrels.len(), "every case needs one qrel row");

    let mut qrels_by_case = BTreeMap::new();
    for qrel in qrels {
        assert_eq!(qrel.schema_version, QREL_SCHEMA, "{}", qrel.case_id);
        let case_id = qrel.case_id.clone();
        assert!(
            qrels_by_case.insert(case_id.clone(), qrel).is_none(),
            "duplicate qrels for {case_id}"
        );
    }

    let mut metrics = DiagnosticMetrics::default();
    for fixture in cases {
        assert_eq!(fixture.schema_version, CASE_SCHEMA, "{}", fixture.case_id);
        let qrel = qrels_by_case
            .remove(&fixture.case_id)
            .unwrap_or_else(|| panic!("missing qrels for {}", fixture.case_id));
        let engine = MemoryEngine::open_in_memory(EngineOptions::default())
            .unwrap_or_else(|error| panic!("{}: open engine: {error}", fixture.case_id));

        for operation in fixture.operations {
            assert_eq!(operation.op, "remember", "{}", operation.record_id);
            let expected_id = operation.request.memory_id.unwrap_or_else(|| {
                panic!(
                    "{}: fixture writes need a stable memory_id",
                    operation.record_id
                )
            });
            let receipt = engine.remember(operation.request).unwrap_or_else(|error| {
                panic!("{} / {}: {error}", fixture.case_id, operation.record_id)
            });
            assert_eq!(
                receipt.memory_ids,
                [expected_id],
                "{} / {} returned an unexpected memory",
                fixture.case_id,
                operation.record_id
            );
        }

        let pack = engine
            .recall(fixture.recall.clone())
            .unwrap_or_else(|error| panic!("{}: recall: {error}", fixture.case_id));
        let ordered_ids = pack
            .hits
            .iter()
            .map(|hit| hit.memory.memory_id.to_string())
            .collect::<Vec<_>>();
        metrics.record(&ordered_ids, &qrel);
        eprintln!(
            "RETRIEVAL_CASE case_id={} hits={:?}",
            fixture.case_id,
            pack.hits
                .iter()
                .map(|hit| (
                    hit.memory.memory_id.to_string(),
                    hit.score,
                    hit.signals.clone(),
                    hit.applicability,
                    hit.memory.revision,
                ))
                .collect::<Vec<_>>()
        );
        case_reports.push(CaseReport {
            case_id: fixture.case_id.clone(),
            ordered_hits: pack
                .hits
                .iter()
                .map(|hit| HitReport {
                    memory_id: hit.memory.memory_id.to_string(),
                    score: hit.score,
                    relevance: *qrel
                        .relevance
                        .get(&hit.memory.memory_id.to_string())
                        .unwrap_or(&0),
                    signals: hit.signals.clone(),
                    applicability: hit.applicability,
                    revision: hit.memory.revision,
                })
                .collect(),
            token_budget: pack.token_budget,
            estimated_tokens: pack.estimated_tokens,
            rendered_bytes: pack.rendered.len(),
            warnings: pack.warnings.clone(),
        });

        for forbidden in &qrel.forbidden {
            let forbidden_id = memory_id(forbidden);
            assert!(
                pack.hits
                    .iter()
                    .all(|hit| hit.memory.memory_id != forbidden_id),
                "{} returned forbidden memory {forbidden}",
                fixture.case_id
            );
            assert!(
                pack.sections
                    .iter()
                    .flat_map(|section| &section.items)
                    .all(|item| item.memory_id != forbidden_id),
                "{} structured context contains forbidden memory {forbidden}",
                fixture.case_id
            );
            assert!(
                !pack.rendered.contains(forbidden),
                "{} rendered forbidden memory {forbidden}",
                fixture.case_id
            );
        }

        for (raw_id, expected_signals) in &qrel.expected_signals {
            let id = memory_id(raw_id);
            let hit = pack
                .hits
                .iter()
                .find(|hit| hit.memory.memory_id == id)
                .unwrap_or_else(|| {
                    panic!(
                        "{} did not return signal-gated memory {raw_id}",
                        fixture.case_id
                    )
                });
            for signal in expected_signals {
                assert!(
                    hit.signals.contains(signal),
                    "{} / {raw_id} lacks signal {signal:?}: {:?}",
                    fixture.case_id,
                    hit.signals
                );
            }
        }

        for (raw_id, expected) in &qrel.expected_applicability {
            let id = memory_id(raw_id);
            let hit = pack
                .hits
                .iter()
                .find(|hit| hit.memory.memory_id == id)
                .unwrap_or_else(|| {
                    panic!(
                        "{} did not return applicability-gated memory {raw_id}",
                        fixture.case_id
                    )
                });
            assert_eq!(
                &hit.applicability, expected,
                "{} / {raw_id}",
                fixture.case_id
            );
        }

        for (raw_id, expected) in &qrel.expected_excluded {
            let id = memory_id(raw_id);
            let memory = engine
                .get(id)
                .unwrap_or_else(|error| panic!("{} / {raw_id}: {error}", fixture.case_id));
            let actual = classify_applicability(
                &memory.scope,
                &fixture.recall.scope,
                &memory.artifacts,
                &fixture.recall.hints.artifacts,
            );
            assert_eq!(&actual, expected, "{} / {raw_id}", fixture.case_id);
        }

        for (raw_id, expected_revision) in &qrel.expected_revision {
            let id = memory_id(raw_id);
            let hit = pack
                .hits
                .iter()
                .find(|hit| hit.memory.memory_id == id)
                .unwrap_or_else(|| {
                    panic!(
                        "{} did not return revision-gated memory {raw_id}",
                        fixture.case_id
                    )
                });
            assert_eq!(
                hit.memory.revision, *expected_revision,
                "{} / {raw_id}",
                fixture.case_id
            );
            for forbidden_text in &qrel.forbidden_body_substrings {
                assert!(
                    !hit.memory.body.contains(forbidden_text),
                    "{} / {raw_id} retained retired text {forbidden_text:?}",
                    fixture.case_id
                );
            }
        }
        for forbidden_text in &qrel.forbidden_body_substrings {
            assert!(
                !pack.rendered.contains(forbidden_text),
                "{} rendered retired text {forbidden_text:?}",
                fixture.case_id
            );
        }

        let ranks = pack
            .hits
            .iter()
            .enumerate()
            .map(|(index, hit)| (hit.memory.memory_id, index))
            .collect::<BTreeMap<_, _>>();
        for [preferred, lower] in &qrel.rank_before {
            let preferred_id = memory_id(preferred);
            let lower_id = memory_id(lower);
            let preferred_rank = ranks.get(&preferred_id).unwrap_or_else(|| {
                panic!(
                    "{} did not return preferred memory {preferred}",
                    fixture.case_id
                )
            });
            let lower_rank = ranks.get(&lower_id).unwrap_or_else(|| {
                panic!(
                    "{} did not return lower-ranked memory {lower}",
                    fixture.case_id
                )
            });
            assert!(
                preferred_rank < lower_rank,
                "{} ranked {preferred} at {preferred_rank} after {lower} at {lower_rank}",
                fixture.case_id
            );
        }
    }

    assert!(
        qrels_by_case.is_empty(),
        "qrels without cases: {:?}",
        qrels_by_case.keys().collect::<Vec<_>>()
    );
    metrics.report();
    if let Some(path) = env::var_os("SUPER_MEM_LONG_HORIZON_REPORT") {
        let report = BenchmarkReport {
            schema_version: "supermem.long_horizon.report.v1",
            fixture_digest,
            commit: env::var("GITHUB_SHA").ok(),
            metrics: metrics.summary(),
            cases: case_reports,
        };
        let encoded = serde_json::to_string_pretty(&report).expect("serialize benchmark report");
        fs::write(&path, format!("{encoded}\n"))
            .unwrap_or_else(|error| panic!("write {}: {error}", Path::new(&path).display()));
    }
}
