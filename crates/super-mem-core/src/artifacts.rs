//! Safe, deterministic fingerprints for repository-relative artifacts.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{ArtifactRef, Error, Result};

const MAX_ARTIFACTS: usize = 128;
const MAX_EXPLICIT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_AUTOMATIC_BYTES: u64 = 64 * 1024 * 1024;
const MAX_GIT_PATH_OUTPUT_BYTES: usize = 1024 * 1024;
const MISSING_CONTENT_HASH: &str = "super-mem:missing:v1";

/// Fingerprints explicit repository-relative paths.
///
/// Paths are resolved below `repository_root` without following symbolic-link
/// components. Missing final paths are represented explicitly so deletions can
/// be revalidated later.
pub fn capture_artifact_paths(
    repository_root: impl AsRef<Path>,
    repo_id: &str,
    paths: &[PathBuf],
) -> Result<Vec<ArtifactRef>> {
    capture_paths(
        repository_root.as_ref(),
        repo_id,
        paths,
        MAX_ARTIFACTS,
        MAX_EXPLICIT_BYTES,
    )
}

/// Best-effort snapshot of every staged, unstaged, deleted, and untracked path.
///
/// Automatic capture returns an empty set when the complete changed-path set
/// cannot be represented within its bounds. A partial set could incorrectly
/// make a checkpoint look current, so it is never returned.
pub fn capture_changed_artifacts(
    repository_root: impl AsRef<Path>,
    repo_id: &str,
) -> Result<Vec<ArtifactRef>> {
    let root = repository_root.as_ref();
    let Some(paths) = changed_paths(root)? else {
        return Ok(Vec::new());
    };
    if paths.len() > MAX_ARTIFACTS {
        return Ok(Vec::new());
    }
    match capture_paths(root, repo_id, &paths, MAX_ARTIFACTS, MAX_AUTOMATIC_BYTES) {
        Ok(artifacts) => Ok(artifacts),
        Err(Error::InvalidInput(_) | Error::Io(_)) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Materializes current hashes for historical artifacts without trusting their
/// stored paths. Failure is uncertainty, so callers receive no inferred hints
/// instead of a false stale/current classification.
pub(crate) fn materialize_current_artifacts(
    repository_root: &Path,
    repo_id: &str,
    historical: &[ArtifactRef],
) -> Vec<ArtifactRef> {
    let paths = historical
        .iter()
        .filter(|artifact| artifact.repo_id == repo_id && artifact.content_hash.is_some())
        .map(|artifact| PathBuf::from(&artifact.path))
        .collect::<Vec<_>>();
    if paths.is_empty() || paths.len() > MAX_ARTIFACTS {
        return Vec::new();
    }
    let Ok(captured) = capture_paths(
        repository_root,
        repo_id,
        &paths,
        MAX_ARTIFACTS,
        MAX_AUTOMATIC_BYTES,
    ) else {
        return Vec::new();
    };
    let by_path = captured
        .into_iter()
        .map(|artifact| (artifact.path, artifact.content_hash))
        .collect::<BTreeMap<_, _>>();
    historical
        .iter()
        .filter_map(|artifact| {
            let content_hash = by_path.get(&artifact.path)?.clone();
            Some(ArtifactRef {
                repo_id: repo_id.to_owned(),
                path: artifact.path.clone(),
                symbol: artifact.symbol.clone(),
                content_hash,
                git_oid: None,
                language: artifact.language.clone(),
            })
        })
        .collect()
}

fn capture_paths(
    repository_root: &Path,
    repo_id: &str,
    paths: &[PathBuf],
    maximum_paths: usize,
    maximum_bytes: u64,
) -> Result<Vec<ArtifactRef>> {
    if repo_id.trim().is_empty() {
        return Err(Error::InvalidInput(
            "artifact capture requires a repository identity".into(),
        ));
    }
    if paths.len() > maximum_paths {
        return Err(Error::InvalidInput(format!(
            "artifact capture contains {} paths; maximum is {maximum_paths}",
            paths.len()
        )));
    }
    let root = repository_root.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::InvalidInput(format!(
            "artifact root {} is not a directory",
            root.display()
        )));
    }

    let mut normalized = BTreeSet::new();
    for path in paths {
        normalized.insert(normalize_relative_path(path)?);
    }

    let mut total_bytes = 0_u64;
    let mut inspected = Vec::with_capacity(normalized.len());
    for (display, relative) in normalized {
        let target = validate_no_symlink_components(&root, &relative)?;
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::InvalidInput(format!(
                    "artifact path {display} is a symbolic link"
                )));
            }
            Ok(metadata) if metadata.is_file() => Some(metadata),
            Ok(_) => {
                return Err(Error::InvalidInput(format!(
                    "artifact path {display} is not a regular file"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(metadata) = &metadata {
            total_bytes = total_bytes.saturating_add(metadata.len());
            if total_bytes > maximum_bytes {
                return Err(Error::InvalidInput(format!(
                    "artifact capture exceeds {maximum_bytes} bytes"
                )));
            }
        }
        inspected.push((display, target, metadata.is_some()));
    }

    inspected
        .into_iter()
        .map(|(path, target, exists)| {
            let content_hash = if exists {
                Some(hash_file(&target)?)
            } else {
                Some(MISSING_CONTENT_HASH.to_owned())
            };
            Ok(ArtifactRef {
                repo_id: repo_id.to_owned(),
                language: language_for_path(&path).map(str::to_owned),
                path,
                content_hash,
                ..ArtifactRef::default()
            })
        })
        .collect()
}

fn normalize_relative_path(path: &Path) -> Result<(String, PathBuf)> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(Error::InvalidInput(
            "artifact paths must be non-empty and repository-relative".into(),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidInput(format!(
                    "artifact path {} must not contain traversal or root components",
                    path.display()
                )));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(Error::InvalidInput(
            "artifact paths must name a repository-relative file".into(),
        ));
    }
    let display = normalized
        .to_str()
        .ok_or_else(|| Error::InvalidInput("artifact paths must be valid UTF-8".into()))?
        .replace('\\', "/");
    Ok((display, normalized))
}

fn validate_no_symlink_components(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::InvalidInput(format!(
                    "artifact path {} contains a symbolic-link component",
                    relative.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(root.join(relative))
}

fn hash_file(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("blake3:{}", hasher.finalize().to_hex()))
}

fn changed_paths(root: &Path) -> Result<Option<Vec<PathBuf>>> {
    let commands: &[&[&str]] = &[
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACDMRTUXB",
            "--cached",
            "--",
        ],
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=ACDMRTUXB",
            "--",
        ],
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    ];
    let mut paths = BTreeSet::new();
    for arguments in commands {
        let output = match Command::new("git")
            .args(*arguments)
            .current_dir(root)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !output.status.success() {
            // Applicability may treat a complete matching artifact set as
            // stronger than a dirty-worktree change. Returning only the
            // command subsets that happened to succeed would therefore turn
            // uncertainty into a false Exact classification.
            return Ok(None);
        }
        if output.stdout.len() > MAX_GIT_PATH_OUTPUT_BYTES {
            return Ok(None);
        }
        for value in output.stdout.split(|byte| *byte == 0).filter(|value| !value.is_empty()) {
            let Some(path) = bytes_to_path(value) else {
                return Ok(None);
            };
            paths.insert(path);
            if paths.len() > MAX_ARTIFACTS {
                return Ok(None);
            }
        }
    }
    Ok(Some(paths.into_iter().collect()))
}

#[cfg(unix)]
fn bytes_to_path(value: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(value.to_vec())))
}

#[cfg(not(unix))]
fn bytes_to_path(value: &[u8]) -> Option<PathBuf> {
    String::from_utf8(value.to_vec()).ok().map(PathBuf::from)
}

fn language_for_path(path: &str) -> Option<&'static str> {
    match Path::new(path).extension().and_then(OsStr::to_str)?.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("cpp"),
        "cs" => Some("csharp"),
        "swift" => Some("swift"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "sh" | "bash" | "zsh" => Some("shell"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "json" => Some("json"),
        "md" | "mdx" => Some("markdown"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repository: &Path, arguments: &[&str]) {
        assert!(
            Command::new("git")
                .args(arguments)
                .current_dir(repository)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn explicit_capture_hashes_files_and_represents_deletions() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(directory.path().join("src/lib.rs"), "pub fn answer() -> u8 { 42 }").unwrap();
        let artifacts = capture_artifact_paths(
            directory.path(),
            "repo",
            &[PathBuf::from("src/lib.rs"), PathBuf::from("src/old.rs")],
        )
        .unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].language.as_deref(), Some("rust"));
        assert!(artifacts[0].content_hash.as_deref().unwrap().starts_with("blake3:"));
        assert_eq!(artifacts[1].content_hash.as_deref(), Some(MISSING_CONTENT_HASH));
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        for path in [PathBuf::from("../outside"), directory.path().join("absolute")] {
            assert!(capture_artifact_paths(directory.path(), "repo", &[path]).is_err());
        }
    }

    #[test]
    fn automatic_capture_includes_the_complete_changed_path_set() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]);
        fs::write(directory.path().join("tracked.rs"), "fn before() {}").unwrap();
        git(directory.path(), &["add", "tracked.rs"]);
        git(
            directory.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ],
        );

        fs::write(directory.path().join("tracked.rs"), "fn after() {}").unwrap();
        fs::write(directory.path().join("untracked.rs"), "fn new() {}").unwrap();
        let artifacts = capture_changed_artifacts(directory.path(), "repo").unwrap();
        assert_eq!(
            artifacts
                .iter()
                .map(|artifact| artifact.path.as_str())
                .collect::<Vec<_>>(),
            ["tracked.rs", "untracked.rs"]
        );
        assert!(
            artifacts
                .iter()
                .all(|artifact| artifact.content_hash.as_deref().is_some_and(|hash| hash.starts_with("blake3:")))
        );

        fs::remove_file(directory.path().join("tracked.rs")).unwrap();
        let artifacts = capture_changed_artifacts(directory.path(), "repo").unwrap();
        let deleted = artifacts
            .iter()
            .find(|artifact| artifact.path == "tracked.rs")
            .unwrap();
        assert_eq!(deleted.content_hash.as_deref(), Some(MISSING_CONTENT_HASH));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_components_are_rejected() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.rs"), "secret").unwrap();
        symlink(outside.path(), directory.path().join("link")).unwrap();
        assert!(
            capture_artifact_paths(
                directory.path(),
                "repo",
                &[PathBuf::from("link/secret.rs")]
            )
            .is_err()
        );
    }
}
