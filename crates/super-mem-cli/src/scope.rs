use std::path::{Path, PathBuf};

use super_mem_core::{RepositoryContext, Scope, canonical_path_digest, discover_repository};

use crate::cli::ScopeArgs;

pub(crate) fn build_scope(arguments: &ScopeArgs) -> Scope {
    let cwd = arguments
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let normalized_cwd = normalize_path(&cwd);

    let mut repository = discover_repository(&cwd).ok().flatten();
    if arguments.repo_id.is_some()
        || arguments.branch.is_some()
        || arguments.head.is_some()
        || arguments.remote.is_some()
    {
        let repo = repository.get_or_insert_with(|| RepositoryContext {
            root: Some(normalized_cwd.clone()),
            ..RepositoryContext::default()
        });
        if let Some(value) = &arguments.repo_id {
            repo.repo_id.clone_from(value);
        }
        if let Some(value) = &arguments.branch {
            repo.branch = Some(value.clone());
        }
        if let Some(value) = &arguments.head {
            repo.head_oid = Some(value.clone());
        }
        if let Some(value) = &arguments.remote {
            repo.remote = Some(value.clone());
        }
    }

    let workspace_id = arguments.workspace.clone().or_else(|| {
        if repository.is_none() {
            Some(format!("path:{}", canonical_path_digest(&cwd)))
        } else {
            None
        }
    });

    Scope {
        namespace: arguments.namespace.clone(),
        workspace_id,
        repository,
        session_id: arguments.session.clone(),
    }
}

pub(crate) fn normalize_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn fallback_workspaces_distinguish_non_utf8_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let directory = tempfile::tempdir().unwrap();
        let first = directory
            .path()
            .join(OsString::from_vec(vec![b'w', b'o', b'r', b'k', 0xfe]));
        let second = directory
            .path()
            .join(OsString::from_vec(vec![b'w', b'o', b'r', b'k', 0xff]));
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let first_scope = build_scope(&ScopeArgs {
            namespace: "default".into(),
            cwd: Some(first),
            ..ScopeArgs::default()
        });
        let second_scope = build_scope(&ScopeArgs {
            namespace: "default".into(),
            cwd: Some(second),
            ..ScopeArgs::default()
        });
        assert!(first_scope.repository.is_none());
        assert!(second_scope.repository.is_none());
        assert_ne!(first_scope.workspace_id, second_scope.workspace_id);
    }
}
