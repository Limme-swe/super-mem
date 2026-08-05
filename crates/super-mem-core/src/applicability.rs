//! Repository, branch, session, and artifact applicability.

use crate::{Applicability, ArtifactRef, GitRelation, Scope, compare_revisions};

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
            (Some(old), Some(now)) if old != now => return Applicability::Stale,
            (Some(_), Some(_)) => verified += 1,
            _ => {}
        }
    }
    let artifact_set_verified = verifiable > 0 && verified == verifiable;

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

    match (
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
}
