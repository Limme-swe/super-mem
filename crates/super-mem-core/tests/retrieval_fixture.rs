//! Production-engine regression runner for the checked-in retrieval fixture.

#![allow(clippy::too_many_lines)]

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, de::DeserializeOwned};
use super_mem_core::{
    Applicability, ContextHints, DenseQuery, EngineOptions, MemoryEngine, MemoryId, RecallRequest,
    RegisterSearchProjectionsRequest, RememberRequest, RetrievalSignal, Scope,
    SearchProfileRegistration, SearchProjectionInput, classify_applicability,
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

    fn report(&self) {
        let cases = f64::from(self.cases.max(1));
        eprintln!(
            "RETRIEVAL_FIXTURE cases={} mrr_at_10={:.6} recall_at_10={:.6} ndcg_at_10={:.6}",
            self.cases,
            self.reciprocal_rank_at_10 / cases,
            self.recall_at_10 / cases,
            self.ndcg_at_10 / cases,
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
        .join("fixtures/retrieval")
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
fn production_engine_satisfies_retrieval_fixture_contract() {
    let Some(case_text) = fixture_text("v1.jsonl") else {
        return;
    };
    let qrel_text = fixture_text("qrels-v1.jsonl")
        .expect("qrels must exist whenever the retrieval cases exist");
    let cases = parse_jsonl::<FixtureCase>("v1.jsonl", &case_text);
    let qrels = parse_jsonl::<FixtureQrels>("qrels-v1.jsonl", &qrel_text);
    assert!(!cases.is_empty(), "retrieval fixture must not be empty");
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
}

#[test]
fn production_engine_registers_and_recalls_dense_projections() {
    let engine = MemoryEngine::open_in_memory(EngineOptions::default()).unwrap();
    let scope = Scope::default();
    let target_id = engine
        .remember(RememberRequest {
            title: "Target procedure".into(),
            body: "Canonical wording shares no query tokens.".into(),
            scope: scope.clone(),
            ..RememberRequest::default()
        })
        .unwrap()
        .memory_ids[0];
    let distractor_id = engine
        .remember(RememberRequest {
            title: "Distractor procedure".into(),
            body: "Another unrelated canonical record.".into(),
            scope: scope.clone(),
            ..RememberRequest::default()
        })
        .unwrap()
        .memory_ids[0];

    let profile = engine
        .register_search_profile(SearchProfileRegistration {
            profile_id: "integration-dense-3d-v1".into(),
            model_digest: "fixture-vector-generator-v1".into(),
            dimensions: Some(3),
        })
        .unwrap();
    assert_eq!(profile.signature_version, 1);
    assert!(profile.active);
    let pending = engine
        .pending_search_documents(&profile.profile_id, scope.clone(), 10)
        .unwrap();
    let projection = |memory_id, vector| {
        let source = pending
            .iter()
            .find(|document| document.memory_id == memory_id)
            .unwrap();
        SearchProjectionInput {
            memory_id,
            revision: source.revision,
            content_hash: source.content_hash.clone(),
            expansions: Vec::new(),
            vector: Some(vector),
        }
    };
    let receipt = engine
        .register_search_projections(RegisterSearchProjectionsRequest {
            scope: scope.clone(),
            profile_id: profile.profile_id.clone(),
            projections: vec![
                projection(target_id, vec![1.0, 0.0, 0.0]),
                projection(distractor_id, vec![0.0, 1.0, 0.0]),
            ],
        })
        .unwrap();
    assert_eq!(receipt.registered, 2);
    let status = engine
        .search_index_status(&profile.profile_id, scope.clone())
        .unwrap();
    assert_eq!((status.eligible, status.indexed, status.pending), (2, 2, 0));

    let pack = engine
        .recall(RecallRequest {
            query: "semantic-only query".into(),
            scope,
            hints: ContextHints {
                dense: Some(DenseQuery {
                    profile_id: profile.profile_id,
                    vector: vec![1.0, 0.0, 0.0],
                    min_similarity: Some(0.5),
                }),
                ..ContextHints::default()
            },
            ..RecallRequest::default()
        })
        .unwrap();
    assert_eq!(pack.hits[0].memory.memory_id, target_id);
    assert!(pack.hits[0].signals.contains(&RetrievalSignal::DenseVector));
}
