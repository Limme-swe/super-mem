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
        None,
    )
}

pub(crate) fn classify_applicability_with_relation(
    memory_scope: &Scope,
    current_scope: &Scope,
    memory_artifacts: &[ArtifactRef],
    current_artifacts: &[ArtifactRef],
    git_relation: Option<GitRelation>,
) -> Applicability {
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

    let mut verified = false;
    for historical in memory_artifacts {
        if historical.repo_id.is_empty() {
            continue;
        }
        for current in current_artifacts {
            if historical.repo_id == current.repo_id
                && historical.path == current.path
                && historical.symbol == current.symbol
            {
                match (&historical.content_hash, &current.content_hash) {
                    (Some(old), Some(now)) if old != now => return Applicability::Stale,
                    (Some(_), Some(_)) => verified = true,
                    _ => {}
                }
            }
        }
    }

    if memory_repo.dirty_hash != current_repo.dirty_hash
        && (memory_repo.dirty_hash.is_some() || current_repo.dirty_hash.is_some())
    {
        return Applicability::Stale;
    }
    if verified {
        return Applicability::Exact;
    }

    match (
        current_repo.root.as_deref(),
        memory_repo.head_oid.as_deref(),
        current_repo.head_oid.as_deref(),
    ) {
        (Some(root), Some(stored), Some(current)) => {
            match git_relation.unwrap_or_else(|| compare_revisions(root, stored, current)) {
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
