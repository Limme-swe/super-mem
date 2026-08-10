//! Repository, branch, session, and artifact applicability.

use crate::{Applicability, ArtifactRef, GitRelation, Scope, compare_revisions};

/// Fixed-width artifact material used while recall candidates are staged.
///
/// Candidate paths and symbols can each be several KiB. Retaining those
/// strings for every oversampled candidate makes recall memory proportional to
/// attacker-controlled metadata size. These domain-separated fingerprints
/// preserve the exact identity/content comparisons used by applicability while
/// keeping each retained artifact at a constant 64 bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactFingerprint {
    identity: [u8; 32],
    content: [u8; 32],
}

/// A bounded set of verifiable artifacts.
///
/// `complete` is false when storage contained more artifacts than candidate
/// staging retained. A partial set can prove a mismatch, but it must never
/// prove that the entire historical artifact set is current.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArtifactFingerprintSet {
    pub(crate) fingerprints: Vec<ArtifactFingerprint>,
    pub(crate) complete: bool,
}

impl ArtifactFingerprintSet {
    pub(crate) fn complete(fingerprints: Vec<ArtifactFingerprint>) -> Self {
        Self {
            fingerprints,
            complete: true,
        }
    }

    pub(crate) fn is_fully_verified_by(&self, current: &Self) -> bool {
        self.complete
            && !self.fingerprints.is_empty()
            && self.fingerprints.iter().all(|historical| {
                current.fingerprints.iter().any(|candidate| {
                    historical.identity == candidate.identity
                        && historical.content == candidate.content
                })
            })
    }
}

pub(crate) fn artifact_fingerprint(
    repo_id: &str,
    path: &str,
    symbol: Option<&str>,
    content_hash: &str,
) -> ArtifactFingerprint {
    let mut identity = blake3::Hasher::new();
    update_length_framed(&mut identity, b"super-mem:artifact-identity:v1");
    update_length_framed(&mut identity, repo_id.as_bytes());
    update_length_framed(&mut identity, path.as_bytes());
    match symbol {
        Some(symbol) => {
            identity.update(&[1]);
            update_length_framed(&mut identity, symbol.as_bytes());
        }
        None => {
            identity.update(&[0]);
        }
    }

    let mut content = blake3::Hasher::new();
    update_length_framed(&mut content, b"super-mem:artifact-content:v1");
    update_length_framed(&mut content, content_hash.as_bytes());
    ArtifactFingerprint {
        identity: *identity.finalize().as_bytes(),
        content: *content.finalize().as_bytes(),
    }
}

pub(crate) fn fingerprint_artifacts(artifacts: &[ArtifactRef]) -> ArtifactFingerprintSet {
    ArtifactFingerprintSet::complete(
        artifacts
            .iter()
            .filter_map(|artifact| {
                artifact.content_hash.as_deref().map(|content_hash| {
                    artifact_fingerprint(
                        &artifact.repo_id,
                        &artifact.path,
                        artifact.symbol.as_deref(),
                        content_hash,
                    )
                })
            })
            .collect(),
    )
}

fn update_length_framed(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArtifactComparison {
    mismatched: bool,
    fully_verified: bool,
    complete: bool,
}

fn compare_artifact_refs(
    memory_artifacts: &[ArtifactRef],
    current_artifacts: &[ArtifactRef],
) -> ArtifactComparison {
    let mut verifiable = 0_usize;
    let mut verified = 0_usize;
    for historical in memory_artifacts
        .iter()
        .filter(|artifact| artifact.content_hash.is_some())
    {
        verifiable += 1;
        let current = current_artifacts.iter().find(|current| {
            historical.repo_id == current.repo_id
                && historical.path == current.path
                && historical.symbol == current.symbol
        });
        match (
            historical.content_hash.as_deref(),
            current.and_then(|artifact| artifact.content_hash.as_deref()),
        ) {
            (Some(old), Some(now)) if old != now => {
                return ArtifactComparison {
                    mismatched: true,
                    fully_verified: false,
                    complete: true,
                };
            }
            (Some(_), Some(_)) => verified += 1,
            _ => {}
        }
    }
    ArtifactComparison {
        mismatched: false,
        fully_verified: verifiable > 0 && verified == verifiable,
        complete: true,
    }
}

fn compare_artifact_fingerprints(
    memory_artifacts: &ArtifactFingerprintSet,
    current_artifacts: &ArtifactFingerprintSet,
) -> ArtifactComparison {
    let mut verified = 0_usize;
    for historical in &memory_artifacts.fingerprints {
        let current = current_artifacts
            .fingerprints
            .iter()
            .find(|current| historical.identity == current.identity);
        if let Some(current) = current {
            if historical.content != current.content {
                return ArtifactComparison {
                    mismatched: true,
                    fully_verified: false,
                    complete: memory_artifacts.complete,
                };
            }
            verified += 1;
        }
    }
    ArtifactComparison {
        mismatched: false,
        fully_verified: memory_artifacts.complete
            && !memory_artifacts.fingerprints.is_empty()
            && verified == memory_artifacts.fingerprints.len(),
        complete: memory_artifacts.complete,
    }
}

/// Classifies whether a memory can safely be applied to the current scope.
///
/// Namespace and repository mismatches are hard isolation boundaries. Branch
/// and session differences make a memory related, while conflicting content
/// hashes make it stale.
pub fn classify_applicability(
    memory_scope: &Scope,
    current_scope: &Scope,
    memory_artifacts: &[ArtifactRef],
    current_artifacts: &[ArtifactRef],
) -> Applicability {
    classify_applicability_with_relation(
        memory_scope,
        current_scope,
        memory_artifacts,
        current_artifacts,
        |root, stored, current| compare_revisions(root, stored, current),
    )
}

pub(crate) fn classify_applicability_with_relation<F>(
    memory_scope: &Scope,
    current_scope: &Scope,
    memory_artifacts: &[ArtifactRef],
    current_artifacts: &[ArtifactRef],
    resolve_git_relation: F,
) -> Applicability
where
    F: FnOnce(&str, &str, &str) -> GitRelation,
{
    classify_applicability_with_artifact_comparison(
        memory_scope,
        current_scope,
        || compare_artifact_refs(memory_artifacts, current_artifacts),
        resolve_git_relation,
    )
}

pub(crate) fn classify_applicability_fingerprints_with_relation<F>(
    memory_scope: &Scope,
    current_scope: &Scope,
    memory_artifacts: &ArtifactFingerprintSet,
    current_artifacts: &ArtifactFingerprintSet,
    resolve_git_relation: F,
) -> Applicability
where
    F: FnOnce(&str, &str, &str) -> GitRelation,
{
    classify_applicability_with_artifact_comparison(
        memory_scope,
        current_scope,
        || compare_artifact_fingerprints(memory_artifacts, current_artifacts),
        resolve_git_relation,
    )
}

fn classify_applicability_with_artifact_comparison<F, A>(
    memory_scope: &Scope,
    current_scope: &Scope,
    compare_artifacts: A,
    resolve_git_relation: F,
) -> Applicability
where
    F: FnOnce(&str, &str, &str) -> GitRelation,
    A: FnOnce() -> ArtifactComparison,
{
    if memory_scope.namespace != current_scope.namespace {
        return Applicability::Inapplicable;
    }

    match (&memory_scope.workspace_id, &current_scope.workspace_id) {
        (Some(memory_workspace), Some(current_workspace))
            if memory_workspace != current_workspace =>
        {
            return Applicability::Inapplicable;
        }
        (Some(_), None) => return Applicability::Inapplicable,
        _ => {}
    }

    if memory_scope.repository.is_some() && current_scope.repository.is_none() {
        return Applicability::Inapplicable;
    }

    let (Some(memory_repo), Some(current_repo)) = (
        memory_scope.repository.as_ref(),
        current_scope.repository.as_ref(),
    ) else {
        return Applicability::Unversioned;
    };

    match (memory_scope.repo_id(), current_scope.repo_id()) {
        (Some(memory_repo), Some(current_repo)) if memory_repo != current_repo => {
            return Applicability::Inapplicable;
        }
        _ => {}
    }

    let artifact_comparison = compare_artifacts();
    if artifact_comparison.mismatched {
        return Applicability::Stale;
    }
    let artifact_set_verified = artifact_comparison.fully_verified;

    if memory_repo.dirty_hash != current_repo.dirty_hash
        && (memory_repo.dirty_hash.is_some() || current_repo.dirty_hash.is_some())
    {
        return if artifact_set_verified {
            Applicability::Exact
        } else {
            Applicability::Stale
        };
    }
    if artifact_set_verified {
        return Applicability::Exact;
    }

    let git_applicability = match (
        current_repo.root.as_deref(),
        memory_repo.head_oid.as_deref(),
        current_repo.head_oid.as_deref(),
    ) {
        (Some(root), Some(stored), Some(current)) => {
            match resolve_git_relation(root, stored, current) {
                GitRelation::Same => Applicability::Exact,
                GitRelation::Ancestor { .. } => Applicability::Compatible,
                GitRelation::Descendant { .. } | GitRelation::Diverged { .. } => {
                    Applicability::Divergent
                }
                GitRelation::Unknown => Applicability::Unversioned,
            }
        }
        _ if matches!(
            (memory_repo.branch.as_deref(), current_repo.branch.as_deref()),
            (Some(memory_branch), Some(current_branch)) if memory_branch != current_branch
        ) =>
        {
            Applicability::Divergent
        }
        _ => Applicability::Unversioned,
    };
    // A truncated historical artifact set may still prove a mismatch, but a
    // matching Git revision cannot prove that omitted attachment hashes are
    // current. Downgrade only Exact; retain stronger stale/divergent evidence.
    if !artifact_comparison.complete && git_applicability == Applicability::Exact {
        Applicability::Unversioned
    } else {
        git_applicability
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepositoryContext;

    fn scope(repo: &str, branch: &str) -> Scope {
        Scope {
            repository: Some(RepositoryContext {
                repo_id: repo.to_owned(),
                branch: Some(branch.to_owned()),
                ..RepositoryContext::default()
            }),
            ..Scope::default()
        }
    }

    #[test]
    fn changed_artifact_is_stale() {
        let old = ArtifactRef {
            repo_id: "repo".into(),
            path: "src/lib.rs".into(),
            content_hash: Some("old".into()),
            ..ArtifactRef::default()
        };
        let new = ArtifactRef {
            content_hash: Some("new".into()),
            ..old.clone()
        };
        assert_eq!(
            classify_applicability(
                &scope("repo", "main"),
                &scope("repo", "main"),
                &[old],
                &[new]
            ),
            Applicability::Stale
        );
    }

    #[test]
    fn another_repository_is_inapplicable() {
        assert_eq!(
            classify_applicability(&scope("one", "main"), &scope("two", "main"), &[], &[]),
            Applicability::Inapplicable
        );
    }

    #[test]
    fn no_repository_context_is_unversioned() {
        assert_eq!(
            classify_applicability(&Scope::default(), &Scope::default(), &[], &[]),
            Applicability::Unversioned
        );
        assert!((Applicability::Exact.ranking_weight() - 1.0).abs() < f64::EPSILON);
        assert!(
            Applicability::Compatible.ranking_weight() > Applicability::Divergent.ranking_weight()
        );
        assert!(Applicability::Inapplicable.ranking_weight().abs() < f64::EPSILON);
    }

    #[test]
    fn clean_memory_against_dirty_worktree_is_stale() {
        let memory = scope("repo", "main");
        let mut current = memory.clone();
        current.repository.as_mut().unwrap().dirty_hash = Some("dirty".into());
        assert_eq!(
            classify_applicability(&memory, &current, &[], &[]),
            Applicability::Stale
        );
    }

    #[test]
    fn complete_artifact_verification_survives_unrelated_dirty_changes() {
        let mut memory = scope("repo", "main");
        let mut current = memory.clone();
        memory.repository.as_mut().unwrap().dirty_hash = Some("before".into());
        current.repository.as_mut().unwrap().dirty_hash = Some("after".into());
        let first = ArtifactRef {
            repo_id: "repo".into(),
            path: "src/lib.rs".into(),
            content_hash: Some("lib-hash".into()),
            ..ArtifactRef::default()
        };
        let second = ArtifactRef {
            path: "tests/integration.rs".into(),
            content_hash: Some("test-hash".into()),
            ..first.clone()
        };

        assert_eq!(
            classify_applicability(
                &memory,
                &current,
                &[first.clone(), second.clone()],
                &[first.clone(), second.clone()]
            ),
            Applicability::Exact
        );
        assert_eq!(
            classify_applicability(&memory, &current, &[first, second], &[]),
            Applicability::Stale
        );
    }

    #[test]
    fn decisive_artifact_and_dirty_checks_do_not_resolve_git() {
        let mut memory = scope("repo", "main");
        let mut current = memory.clone();
        for candidate in [&mut memory, &mut current] {
            let repository = candidate.repository.as_mut().unwrap();
            repository.root = Some("/repository".into());
            repository.head_oid = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        }
        let historical = ArtifactRef {
            repo_id: "repo".into(),
            path: "src/lib.rs".into(),
            content_hash: Some("old".into()),
            ..ArtifactRef::default()
        };
        let changed = ArtifactRef {
            content_hash: Some("new".into()),
            ..historical.clone()
        };

        let mut calls = 0;
        assert_eq!(
            classify_applicability_with_relation(
                &memory,
                &current,
                std::slice::from_ref(&historical),
                std::slice::from_ref(&changed),
                |_, _, _| {
                    calls += 1;
                    GitRelation::Same
                },
            ),
            Applicability::Stale
        );
        assert_eq!(calls, 0);

        current.repository.as_mut().unwrap().dirty_hash = Some("dirty".into());
        assert_eq!(
            classify_applicability_with_relation(&memory, &current, &[], &[], |_, _, _| {
                calls += 1;
                GitRelation::Same
            }),
            Applicability::Stale
        );
        assert_eq!(calls, 0);

        current.repository.as_mut().unwrap().dirty_hash = None;
        assert_eq!(
            classify_applicability_with_relation(
                &memory,
                &current,
                std::slice::from_ref(&historical),
                std::slice::from_ref(&historical),
                |_, _, _| {
                    calls += 1;
                    GitRelation::Same
                },
            ),
            Applicability::Exact
        );
        assert_eq!(calls, 0);

        assert_eq!(
            classify_applicability_with_relation(&memory, &current, &[], &[], |_, _, _| {
                calls += 1;
                GitRelation::Ancestor { behind: 1 }
            }),
            Applicability::Compatible
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn differing_unresolved_branches_are_divergent() {
        assert_eq!(
            classify_applicability(&scope("repo", "main"), &scope("repo", "feature"), &[], &[]),
            Applicability::Divergent
        );
    }

    #[test]
    fn workspace_scope_is_directional_and_isolated() {
        let memory = Scope {
            workspace_id: Some("workspace-a".into()),
            ..Scope::default()
        };
        let other = Scope {
            workspace_id: Some("workspace-b".into()),
            ..Scope::default()
        };
        assert_eq!(
            classify_applicability(&memory, &other, &[], &[]),
            Applicability::Inapplicable
        );
        assert_eq!(
            classify_applicability(&memory, &Scope::default(), &[], &[]),
            Applicability::Inapplicable
        );
    }

    #[test]
    fn unscoped_callers_cannot_apply_repository_memory() {
        assert_eq!(
            classify_applicability(&scope("repo", "main"), &Scope::default(), &[], &[]),
            Applicability::Inapplicable
        );
    }

    #[test]
    fn fixed_width_fingerprints_preserve_full_artifact_classification() {
        assert_eq!(std::mem::size_of::<ArtifactFingerprint>(), 64);
        let historical = ArtifactRef {
            repo_id: "repo".into(),
            path: "src/large.rs".into(),
            symbol: Some("worker::run".into()),
            content_hash: Some("before".into()),
            ..ArtifactRef::default()
        };
        let matching = historical.clone();
        let changed = ArtifactRef {
            content_hash: Some("after".into()),
            ..historical.clone()
        };

        let cases = [
            (
                vec![historical.clone()],
                vec![matching],
                GitRelation::Diverged {
                    ahead: 1,
                    behind: 1,
                },
            ),
            (vec![historical.clone()], vec![changed], GitRelation::Same),
            (
                vec![historical.clone()],
                Vec::new(),
                GitRelation::Ancestor { behind: 1 },
            ),
            (Vec::new(), Vec::new(), GitRelation::Unknown),
        ];
        for (memory_artifacts, current_artifacts, relation) in cases {
            let mut memory_scope = scope("repo", "main");
            let mut current_scope = scope("repo", "main");
            let memory_repo = memory_scope.repository.as_mut().unwrap();
            memory_repo.root = Some("/repository".into());
            memory_repo.head_oid = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
            let current_repo = current_scope.repository.as_mut().unwrap();
            current_repo.root = Some("/repository".into());
            current_repo.head_oid = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
            let expected = classify_applicability_with_relation(
                &memory_scope,
                &current_scope,
                &memory_artifacts,
                &current_artifacts,
                |_, _, _| relation,
            );
            let memory_fingerprints = fingerprint_artifacts(&memory_artifacts);
            let current_fingerprints = fingerprint_artifacts(&current_artifacts);
            let actual = classify_applicability_fingerprints_with_relation(
                &memory_scope,
                &current_scope,
                &memory_fingerprints,
                &current_fingerprints,
                |_, _, _| relation,
            );
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn incomplete_fingerprint_sets_cannot_override_dirty_state() {
        let mut memory = scope("repo", "main");
        let mut current = memory.clone();
        for candidate in [&mut memory, &mut current] {
            let repository = candidate.repository.as_mut().unwrap();
            repository.root = Some("/repository".into());
            repository.head_oid = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        }
        memory.repository.as_mut().unwrap().dirty_hash = Some("before".into());
        current.repository.as_mut().unwrap().dirty_hash = Some("after".into());
        let artifact = ArtifactRef {
            repo_id: "repo".into(),
            path: "src/lib.rs".into(),
            content_hash: Some("same".into()),
            ..ArtifactRef::default()
        };
        let mut historical = fingerprint_artifacts(std::slice::from_ref(&artifact));
        let current_artifacts = fingerprint_artifacts(std::slice::from_ref(&artifact));
        assert_eq!(
            classify_applicability_fingerprints_with_relation(
                &memory,
                &current,
                &historical,
                &current_artifacts,
                |_, _, _| GitRelation::Same,
            ),
            Applicability::Exact
        );

        historical.complete = false;
        assert!(!historical.is_fully_verified_by(&current_artifacts));
        assert_eq!(
            classify_applicability_fingerprints_with_relation(
                &memory,
                &current,
                &historical,
                &current_artifacts,
                |_, _, _| GitRelation::Same,
            ),
            Applicability::Stale
        );
        current.repository.as_mut().unwrap().dirty_hash = Some("before".into());
        assert_eq!(
            classify_applicability_fingerprints_with_relation(
                &memory,
                &current,
                &historical,
                &current_artifacts,
                |_, _, _| GitRelation::Same,
            ),
            Applicability::Unversioned,
            "an incomplete historical set must not become exact through Git fallback"
        );
    }

    #[test]
    fn fixed_width_fingerprints_preserve_dirty_exact_stale_and_missing_results() {
        let mut memory = scope("repo", "main");
        let mut current = memory.clone();
        memory.repository.as_mut().unwrap().dirty_hash = Some("before".into());
        current.repository.as_mut().unwrap().dirty_hash = Some("after".into());
        let historical = ArtifactRef {
            repo_id: "repo".into(),
            path: "src/lib.rs".into(),
            content_hash: Some("old".into()),
            ..ArtifactRef::default()
        };
        let cases = [
            (vec![historical.clone()], Applicability::Exact),
            (
                vec![ArtifactRef {
                    content_hash: Some("new".into()),
                    ..historical.clone()
                }],
                Applicability::Stale,
            ),
            (Vec::new(), Applicability::Stale),
        ];
        for (current_artifacts, expected) in cases {
            let full = classify_applicability_with_relation(
                &memory,
                &current,
                std::slice::from_ref(&historical),
                &current_artifacts,
                |_, _, _| GitRelation::Same,
            );
            let fingerprints = classify_applicability_fingerprints_with_relation(
                &memory,
                &current,
                &fingerprint_artifacts(std::slice::from_ref(&historical)),
                &fingerprint_artifacts(&current_artifacts),
                |_, _, _| GitRelation::Same,
            );
            assert_eq!(full, expected);
            assert_eq!(fingerprints, full);
        }
    }
}
