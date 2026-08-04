//! Deterministic candidate fusion, scoring, and diversity selection.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::{Applicability, Memory, MemoryKind, MemoryState, RecallHit, RetrievalSignal};

const RRF_K: f64 = 60.0;

#[derive(Clone, Debug, Default)]
pub(crate) struct Candidate {
    pub(crate) ranks: HashMap<RetrievalSignal, usize>,
}

impl Candidate {
    pub(crate) fn record(&mut self, signal: RetrievalSignal, rank: usize) {
        self.ranks
            .entry(signal)
            .and_modify(|existing| *existing = (*existing).min(rank))
            .or_insert(rank);
    }

    pub(crate) fn preliminary_score(&self) -> f64 {
        self.ranks
            .iter()
            .map(|(signal, rank)| source_weight(*signal) / (RRF_K + *rank as f64))
            .sum()
    }
}

pub(crate) fn safe_fts_query(query: &str) -> Option<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in query.chars() {
        if character.is_alphanumeric() || character == '_' {
            current.extend(character.to_lowercase());
            if current.len() >= 64 {
                push_term(&mut terms, &mut current);
            }
        } else {
            push_term(&mut terms, &mut current);
        }
        if terms.len() >= 24 {
            break;
        }
    }
    push_term(&mut terms, &mut current);
    terms.truncate(24);
    terms.dedup();
    if terms.is_empty() {
        None
    } else {
        Some(
            terms
                .into_iter()
                .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }
}

fn push_term(terms: &mut Vec<String>, current: &mut String) {
    let operator = matches!(current.as_str(), "and" | "or" | "not" | "near");
    if current.len() >= 2 && !operator && !terms.iter().any(|existing| existing == current) {
        terms.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

pub(crate) fn score_candidate(
    memory: Memory,
    candidate: &Candidate,
    applicability: Applicability,
    feedback_utility: f64,
    now: DateTime<Utc>,
) -> RecallHit {
    let mut signals = candidate.ranks.keys().copied().collect::<Vec<_>>();
    signals.sort_by_key(|signal| signal_order(*signal));
    let mut base = 0.0;
    for (signal, rank) in &candidate.ranks {
        base += source_weight(*signal) / (RRF_K + *rank as f64);
    }

    let quality = 0.75 + f64::from(memory.importance) * 0.15 + f64::from(memory.confidence) * 0.10;
    let state = match memory.state {
        MemoryState::Active => 1.0,
        MemoryState::Contested => 0.78,
        MemoryState::Superseded => 0.42,
        MemoryState::Retracted => 0.0,
    };
    let temporal = temporal_factor(memory.kind, memory.updated_at, now);
    let utility = (1.0 + feedback_utility.clamp(-0.10, 0.10)).max(0.5);
    let score = base
        * quality
        * state
        * temporal
        * memory.trust.factor()
        * applicability.ranking_weight()
        * utility;

    let mut reasons = Vec::new();
    for signal in &signals {
        reasons.push(match signal {
            RetrievalSignal::Exact => "exact_text".to_owned(),
            RetrievalSignal::Lexical => "lexical_match".to_owned(),
            RetrievalSignal::Sparse => "identifier_or_artifact".to_owned(),
            RetrievalSignal::Entity => "entity_match".to_owned(),
            RetrievalSignal::Recency => "recent_in_scope".to_owned(),
            RetrievalSignal::ArtifactVerified => "artifact_hash_verified".to_owned(),
            RetrievalSignal::ErrorFingerprint => "same_error_fingerprint".to_owned(),
        });
    }
    reasons.push(
        match applicability {
            Applicability::Exact => "exact_repository_state",
            Applicability::Compatible => "compatible_repository_history",
            Applicability::Stale => "artifact_or_worktree_changed",
            Applicability::Divergent => "divergent_repository_history",
            Applicability::Unversioned => "unversioned_scope",
            Applicability::Inapplicable => "inapplicable_scope",
        }
        .to_owned(),
    );

    RecallHit {
        memory,
        score,
        applicability,
        signals,
        reasons,
    }
}

fn source_weight(signal: RetrievalSignal) -> f64 {
    match signal {
        RetrievalSignal::Exact => 1.35,
        RetrievalSignal::Lexical => 1.00,
        RetrievalSignal::Sparse => 0.82,
        RetrievalSignal::Entity => 0.72,
        RetrievalSignal::Recency => 0.20,
        RetrievalSignal::ArtifactVerified => 1.10,
        RetrievalSignal::ErrorFingerprint => 1.25,
    }
}

fn signal_order(signal: RetrievalSignal) -> u8 {
    match signal {
        RetrievalSignal::Exact => 0,
        RetrievalSignal::ErrorFingerprint => 1,
        RetrievalSignal::ArtifactVerified => 2,
        RetrievalSignal::Lexical => 3,
        RetrievalSignal::Sparse => 4,
        RetrievalSignal::Entity => 5,
        RetrievalSignal::Recency => 6,
    }
}

fn temporal_factor(kind: MemoryKind, updated: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    let Some(half_life_days) = (match kind {
        MemoryKind::Constraint | MemoryKind::Decision => None,
        MemoryKind::Preference => Some(365.0),
        MemoryKind::Procedure => Some(180.0),
        MemoryKind::Fact => Some(120.0),
        MemoryKind::Outcome => Some(90.0),
        MemoryKind::Episode => Some(45.0),
        MemoryKind::Task => Some(30.0),
        MemoryKind::Observation => Some(14.0),
    }) else {
        return 1.0;
    };
    let age_days = now.signed_duration_since(updated).num_seconds().max(0) as f64 / 86_400.0;
    0.25 + 0.75 * 2.0_f64.powf(-age_days / half_life_days)
}

pub(crate) fn select_mmr(mut hits: Vec<RecallHit>, limit: usize, lambda: f64) -> Vec<RecallHit> {
    if hits.len() <= 1 || limit == 0 {
        hits.truncate(limit);
        return hits;
    }
    hits.sort_by(|left, right| right.score.total_cmp(&left.score));
    let max_score = hits.first().map_or(1.0, |hit| hit.score.max(f64::EPSILON));
    let token_sets = hits
        .iter()
        .map(|hit| token_set(&format!("{} {}", hit.memory.title, hit.memory.body)))
        .collect::<Vec<_>>();

    let mut selected_indices = Vec::with_capacity(limit.min(hits.len()));
    let mut available = (0..hits.len()).collect::<Vec<_>>();
    while selected_indices.len() < limit && !available.is_empty() {
        let mut best_position = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (position, &candidate_index) in available.iter().enumerate() {
            let redundancy = selected_indices
                .iter()
                .map(|selected| jaccard(&token_sets[candidate_index], &token_sets[*selected]))
                .fold(0.0, f64::max);
            let relevance = hits[candidate_index].score / max_score;
            let mmr = lambda * relevance - (1.0 - lambda) * redundancy;
            if mmr > best_score {
                best_score = mmr;
                best_position = position;
            }
        }
        selected_indices.push(available.swap_remove(best_position));
    }

    selected_indices
        .into_iter()
        .map(|index| hits[index].clone())
        .collect()
}

fn token_set(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| token.len() > 1)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn jaccard(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count();
    let union = left.len() + right.len() - intersection;
    intersection as f64 / union.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_builder_never_passes_operators_through() {
        let query = safe_fts_query(r#"foo' OR 1=1 NOT (bar*) \"baz"#).unwrap();
        assert_eq!(query, "\"foo\" OR \"bar\" OR \"baz\"");
        assert!(!query.contains('*'));
        assert!(!query.contains("NOT"));
    }

    #[test]
    fn fts_builder_bounds_pathological_input() {
        let query = (0..100)
            .map(|index| format!("term{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(safe_fts_query(&query).unwrap().split(" OR ").count(), 24);
    }
}
