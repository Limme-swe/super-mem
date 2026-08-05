//! Small, dependency-free Git discovery and revision comparison helpers.

use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use crate::{GitRelation, RepositoryContext};

/// Discovers repository identity and current state below `path`.
///
/// `Ok(None)` is returned when Git is absent, `path` is outside a repository,
/// or the repository cannot be inspected. Repository discovery is an optional
/// relevance improvement and must not make the memory engine unavailable.
pub fn discover_repository(path: impl AsRef<Path>) -> std::io::Result<Option<RepositoryContext>> {
    let path = path.as_ref();
    let Some(root_path) = git_path(path, &["rev-parse", "--show-toplevel"])? else {
        return Ok(None);
    };
    let root = normalize_path(&root_path);

    let common_raw = git_path(&root_path, &["rev-parse", "--git-common-dir"])?;
    let common_dir_path = common_raw.map(|common| {
        let absolute = if common.is_absolute() {
            common
        } else {
            root_path.join(common)
        };
        absolute.canonicalize().unwrap_or(absolute)
    });
    let common_dir = common_dir_path.as_deref().map(normalize_path);

    let status_output = git_stdout_bounded(
        &root_path,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ],
        STATUS_LIMIT + STATUS_METADATA_ALLOWANCE,
    )?;
    let (branch, head_oid, status, status_truncated) = status_output.map_or_else(
        || (None, None, Vec::new(), false),
        |(output, truncated)| parse_status_metadata(&output, truncated),
    );
    let remote_raw = git_text(&root_path, &["config", "--get", "remote.origin.url"])?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let local_remote = remote_raw
        .as_deref()
        .is_some_and(remote_is_local_repository);
    let remote = remote_raw
        .as_deref()
        .and_then(|value| normalize_discovered_remote(value, &root_path));
    let dirty_hash = dirty_worktree_hash(&root_path, &status, status_truncated);
    // Linked worktrees have distinct roots but share a common Git directory.
    // Prefer it for local repositories so they resolve to one repository ID.
    let repo_id = if local_remote {
        canonical_path_digest(common_dir_path.as_deref().unwrap_or(&root_path))
    } else if let Some(remote) = remote.as_deref() {
        blake3::hash(remote.as_bytes()).to_hex().to_string()
    } else {
        canonical_path_digest(common_dir_path.as_deref().unwrap_or(&root_path))
    };

    Ok(Some(RepositoryContext {
        repo_id,
        root: Some(root),
        common_dir,
        branch,
        head_oid,
        remote,
        dirty_hash,
    }))
}

/// Returns a canonical, native-byte-safe digest for a filesystem path.
///
/// On Unix this hashes the raw `OsStr` bytes, so distinct non-UTF-8 paths
/// cannot collapse through lossy display conversion.
pub fn canonical_path_digest(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    let canonical = path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        }
    });
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"super-mem:canonical-path:v1\0");
    hash_native_path(&mut hasher, &canonical);
    hasher.finalize().to_hex().to_string()
}

/// Sanitizes common Git remote forms into a stable, credential-free identity.
pub fn normalize_remote(remote: &str) -> Option<String> {
    let mut value = remote.trim().replace('\\', "/");
    if value.is_empty() {
        return None;
    }

    if let Some((prefix, rest)) = value.split_once("://") {
        let scheme = prefix.to_ascii_lowercase();
        let without_fragment = rest.split(['?', '#']).next().unwrap_or(rest);
        let authority_end = without_fragment.find('/').unwrap_or(without_fragment.len());
        let (authority, path) = without_fragment.split_at(authority_end);
        let host = authority
            .rsplit('@')
            .next()
            .unwrap_or(authority)
            .to_ascii_lowercase();
        value = if scheme == "file" {
            format!("file://{host}{path}")
        } else {
            format!("{host}{path}")
        };
    } else if let Some((authority, path)) = value.split_once(':') {
        // SCP-like `git@host:owner/repo.git`. Avoid treating Windows drive
        // letters as hosts.
        if authority.len() > 1
            && !authority.contains(['/', '\\'])
            && !(authority.len() == 1 && authority.as_bytes()[0].is_ascii_alphabetic())
        {
            let host = authority
                .rsplit('@')
                .next()
                .unwrap_or(authority)
                .to_ascii_lowercase();
            value = format!("{host}/{path}");
        }
    }

    while value.ends_with('/') {
        value.pop();
    }
    if value.to_ascii_lowercase().ends_with(".git") {
        value.truncate(value.len() - 4);
    }
    Some(value)
}

fn normalize_discovered_remote(remote: &str, repository_root: &Path) -> Option<String> {
    if !remote_is_local_repository(remote)
        || remote.trim().to_ascii_lowercase().starts_with("file://")
    {
        return normalize_remote(remote);
    }
    let path = PathBuf::from(remote.trim());
    #[cfg(windows)]
    if windows_drive_relative(remote.trim()) {
        // `C:relative` is relative to that drive's process-local current
        // directory, not to `repository_root`. Preserve a stable opaque form
        // instead of resolving it against whichever drive cwd launched us.
        return normalize_remote(remote);
    }
    let absolute = if path.is_absolute() {
        path
    } else {
        repository_root.join(path)
    };
    Some(format!("file://{}", normalize_path(&absolute)))
}

#[cfg(windows)]
fn windows_drive_relative(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || !matches!(bytes[2], b'/' | b'\\'))
}

fn remote_is_local_repository(remote: &str) -> bool {
    let value = remote.trim();
    if let Some(rest) = value
        .strip_prefix("file://")
        .or_else(|| value.strip_prefix("FILE://"))
    {
        let authority = rest.split('/').next().unwrap_or_default();
        return authority.is_empty() || authority.eq_ignore_ascii_case("localhost");
    }
    if value.contains("://") {
        return false;
    }
    if let Some((authority, path)) = value.split_once(':') {
        let windows_drive = authority.len() == 1
            && authority.as_bytes()[0].is_ascii_alphabetic()
            && (cfg!(windows) || path.starts_with(['/', '\\']));
        if !windows_drive && !authority.contains(['/', '\\']) && !path.is_empty() {
            return false;
        }
    }
    true
}

/// Compares two validated Git commit IDs without interpreting them as options.
///
/// Invalid IDs, missing Git, shallow-history gaps, and command failures all
/// produce [`GitRelation::Unknown`].
pub fn compare_revisions(
    repository_root: impl AsRef<Path>,
    stored_oid: &str,
    current_oid: &str,
) -> GitRelation {
    if !valid_oid(stored_oid) || !valid_oid(current_oid) {
        return GitRelation::Unknown;
    }
    if stored_oid.eq_ignore_ascii_case(current_oid) {
        return GitRelation::Same;
    }

    let range = format!("{stored_oid}...{current_oid}");
    let Some(counts) = git_text_lossy(
        repository_root.as_ref(),
        &["rev-list", "--left-right", "--count", &range],
    ) else {
        return GitRelation::Unknown;
    };
    let mut numbers = counts.split_whitespace();
    let Some(ahead) = numbers.next().and_then(|value| value.parse::<u32>().ok()) else {
        return GitRelation::Unknown;
    };
    let Some(behind) = numbers.next().and_then(|value| value.parse::<u32>().ok()) else {
        return GitRelation::Unknown;
    };

    match (ahead, behind) {
        (0, 0) => GitRelation::Same,
        (0, behind) => GitRelation::Ancestor { behind },
        (ahead, 0) => GitRelation::Descendant { ahead },
        (ahead, behind) => GitRelation::Diverged { ahead, behind },
    }
}

fn valid_oid(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_text(path: &Path, arguments: &[&str]) -> std::io::Result<Option<String>> {
    match Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
    {
        Ok(output) => Ok(output_text(output)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn git_path(path: &Path, arguments: &[&str]) -> std::io::Result<Option<PathBuf>> {
    let output = match Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = output.stdout;
    // `rev-parse` appends exactly one record terminator. On Unix, additional
    // LF/CR bytes immediately before it are legal path bytes and must remain
    // part of the repository identity.
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    #[cfg(not(unix))]
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        Ok(Some(PathBuf::from(OsString::from_vec(bytes))))
    }
    #[cfg(not(unix))]
    {
        Ok(String::from_utf8(bytes).ok().map(PathBuf::from))
    }
}

fn git_text_lossy(path: &Path, arguments: &[&str]) -> Option<String> {
    Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
        .ok()
        .and_then(output_text)
}

fn output_text(output: Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

const STATUS_LIMIT: u64 = 8 * 1_048_576;
const STATUS_METADATA_ALLOWANCE: u64 = 64 * 1_024;

fn parse_status_metadata(
    output: &[u8],
    command_truncated: bool,
) -> (Option<String>, Option<String>, Vec<u8>, bool) {
    let mut branch = None;
    let mut head_oid = None;
    let mut status = Vec::with_capacity(output.len().min(64 * 1024));
    for record in output.split_inclusive(|byte| *byte == 0) {
        if let Some(value) = status_header(record, b"# branch.head ") {
            branch = std::str::from_utf8(value)
                .ok()
                .map(str::to_owned)
                .filter(|value| value != "(detached)" && !value.is_empty());
        } else if let Some(value) = status_header(record, b"# branch.oid ") {
            head_oid = std::str::from_utf8(value)
                .ok()
                .map(str::to_ascii_lowercase)
                .filter(|value| valid_oid(value));
        } else if record.starts_with(b"# branch.") {
            // Upstream and ahead/behind headers are metadata, not worktree
            // changes. Keeping them would falsely mark a clean tracking
            // branch dirty.
        } else {
            status.extend_from_slice(record);
        }
    }
    let status_truncated = command_truncated || status.len() as u64 > STATUS_LIMIT;
    status.truncate(usize::try_from(STATUS_LIMIT).unwrap_or(usize::MAX));
    (branch, head_oid, status, status_truncated)
}

fn status_header<'a>(record: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    record
        .strip_prefix(prefix)
        .map(|value| value.strip_suffix(&[0]).unwrap_or(value))
}

fn dirty_worktree_hash(root: &Path, status: &[u8], status_truncated: bool) -> Option<String> {
    if status.is_empty() && !status_truncated {
        return None;
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(status);
    if status_truncated {
        hasher.update(b"[status-truncated]");
    }
    hasher.update(&[0]);
    hash_git_stdout_bounded(
        root,
        &["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
        &mut hasher,
        8 * 1_048_576,
    );
    hash_untracked_files(root, &mut hasher);
    Some(hasher.finalize().to_hex().to_string())
}

fn hash_git_stdout_bounded(
    root: &Path,
    arguments: &[&str],
    hasher: &mut blake3::Hasher,
    maximum: u64,
) {
    if let Ok(Some((output, truncated))) = git_stdout_bounded(root, arguments, maximum) {
        hasher.update(&output);
        if truncated {
            hasher.update(b"[tracked-diff-truncated]");
        }
        hasher.update(&[0]);
    }
}

fn git_stdout_bounded(
    root: &Path,
    arguments: &[&str],
    maximum: u64,
) -> std::io::Result<Option<(Vec<u8>, bool)>> {
    let mut child = match Command::new("git")
        .args(arguments)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(None);
    };
    let mut output = Vec::new();
    let mut reader = stdout.take(maximum.saturating_add(1));
    if reader.read_to_end(&mut output).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(None);
    }
    let truncated = output.len() as u64 > maximum;
    if truncated {
        output.truncate(usize::try_from(maximum).unwrap_or(usize::MAX));
        let _ = child.kill();
        let _ = child.wait();
    } else if !child.wait()?.success() {
        return Ok(None);
    }
    Ok(Some((output, truncated)))
}

fn hash_untracked_files(root: &Path, hasher: &mut blake3::Hasher) {
    const MAX_FILE_BYTES: u64 = 1_048_576;
    const MAX_TOTAL_BYTES: u64 = 8_388_608;
    let Ok(Some((output, listing_truncated))) = git_stdout_bounded(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        8 * 1_048_576,
    ) else {
        return;
    };
    let mut total_hashed = 0_u64;
    for encoded_path in output.split(|byte| *byte == 0) {
        if encoded_path.is_empty() {
            continue;
        }
        hasher.update(encoded_path);
        hasher.update(&[0]);
        let Some(path) = join_raw_git_path(root, encoded_path) else {
            hasher.update(b"[unrepresentable-path]");
            continue;
        };
        let Ok(metadata) = path.symlink_metadata() else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            if let Ok(target) = path.read_link() {
                hash_os_path(hasher, &target);
            }
        } else if metadata.is_file() {
            hasher.update(&metadata.len().to_le_bytes());
            if let Ok(mut file) = std::fs::File::open(path) {
                let mut buffer = [0_u8; 16 * 1024];
                let budget = MAX_TOTAL_BYTES.saturating_sub(total_hashed);
                if budget == 0 {
                    hasher.update(b"[total-content-cap]");
                } else {
                    let per_file = MAX_FILE_BYTES.min(budget);
                    let head_budget = if metadata.len() > per_file {
                        per_file / 2
                    } else {
                        per_file
                    };
                    let head = hash_reader_prefix(&mut file, hasher, &mut buffer, head_budget);
                    total_hashed = total_hashed.saturating_add(head);
                    if metadata.len() > per_file {
                        hasher.update(b"[truncated-middle]");
                        let tail_budget = per_file.saturating_sub(head);
                        if tail_budget > 0
                            && file
                                .seek(SeekFrom::End(
                                    -i64::try_from(tail_budget).unwrap_or(i64::MAX),
                                ))
                                .is_ok()
                        {
                            let tail =
                                hash_reader_prefix(&mut file, hasher, &mut buffer, tail_budget);
                            total_hashed = total_hashed.saturating_add(tail);
                        }
                    }
                }
            }
        }
        hasher.update(&[0]);
    }
    if listing_truncated {
        hasher.update(b"[untracked-listing-truncated]");
    }
}

#[allow(clippy::unnecessary_wraps)]
fn join_raw_git_path(root: &Path, encoded_path: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        Some(root.join(OsStr::from_bytes(encoded_path)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(encoded_path.to_vec())
            .ok()
            .map(|relative| root.join(relative))
    }
}

fn hash_os_path(hasher: &mut blake3::Hasher, path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(not(unix))]
    hasher.update(path.to_string_lossy().as_bytes());
}

fn hash_native_path(hasher: &mut blake3::Hasher, path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            hasher.update(&unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    hasher.update(path.to_string_lossy().as_bytes());
}

fn hash_reader_prefix(
    reader: &mut impl Read,
    hasher: &mut blake3::Hasher,
    buffer: &mut [u8],
    mut remaining: u64,
) -> u64 {
    let initial = remaining;
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let Ok(read) = reader.read(&mut buffer[..wanted]) else {
            break;
        };
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        remaining = remaining.saturating_sub(read as u64);
    }
    initial - remaining
}

fn normalize_path(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        // Canonical Windows paths can carry a verbatim `\\?\` prefix. Replacing
        // its separators produces `//?/...`, which is not a round-trippable
        // native path and makes artifact freshness silently unverifiable.
        path.to_string_lossy().into_owned()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().replace('\\', "/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(path: &Path, arguments: &[&str]) -> bool {
        Command::new("git")
            .args(arguments)
            .current_dir(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn sanitizes_remote_credentials_and_transport() {
        assert_eq!(
            normalize_remote("https://user:token@GitHub.COM/Org/Repo.git?x=1"),
            Some("github.com/Org/Repo".into())
        );
        assert_eq!(
            normalize_remote("git@github.com:Org/Repo.git"),
            Some("github.com/Org/Repo".into())
        );
    }

    #[test]
    fn preserves_case_sensitive_remote_paths() {
        assert_ne!(
            normalize_remote("https://example.test/Org/Repo.git"),
            normalize_remote("https://example.test/org/repo.git")
        );
    }

    #[cfg(windows)]
    #[test]
    fn drive_relative_remote_is_local_on_windows() {
        assert!(remote_is_local_repository(r"C:relative\origin.git"));
        assert!(remote_is_local_repository(r"C:\absolute\origin.git"));
        assert_eq!(
            normalize_discovered_remote(r"C:relative\origin.git", Path::new(r"C:\work\repository"),),
            Some("C:relative/origin".into())
        );
        assert_eq!(
            normalize_discovered_remote(r"C:", Path::new(r"C:\work\repository")),
            Some("C:".into())
        );
    }

    #[test]
    fn preserves_file_remote_authority() {
        assert_eq!(
            normalize_remote("file://BuildHost/Org/Repo.git"),
            Some("file://buildhost/Org/Repo".into())
        );
    }

    #[cfg(windows)]
    #[test]
    fn discovered_windows_root_round_trips_as_a_native_path() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repository with spaces");
        std::fs::create_dir(&repository).unwrap();
        if !run_git(&repository, &["init", "--quiet"]) {
            return;
        }

        let discovered = discover_repository(&repository)
            .unwrap()
            .expect("repository");
        let stored = PathBuf::from(discovered.root.expect("root"));
        assert!(stored.is_dir());
        assert_eq!(
            stored.canonicalize().unwrap(),
            repository.canonicalize().unwrap()
        );
    }

    #[test]
    fn porcelain_metadata_is_removed_without_changing_status_bytes() {
        let input = b"# branch.oid AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -0\0? untracked file\0second-path-of-rename\0";
        let (branch, head, status, truncated) = parse_status_metadata(input, false);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(
            head.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(status, b"? untracked file\0second-path-of-rename\0");
        assert!(!truncated);
    }

    #[test]
    fn discovery_gets_branch_head_and_dirty_state_from_one_status_snapshot() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        assert!(run_git(
            directory.path(),
            &["init", "--initial-branch=main"]
        ));
        assert!(run_git(
            directory.path(),
            &["config", "user.email", "tests@example.test"]
        ));
        assert!(run_git(
            directory.path(),
            &["config", "user.name", "Super Mem Tests"]
        ));
        std::fs::write(directory.path().join("tracked.txt"), "initial\n").unwrap();
        assert!(run_git(directory.path(), &["add", "tracked.txt"]));
        assert!(run_git(directory.path(), &["commit", "-m", "initial"]));
        assert!(run_git(
            directory.path(),
            &["config", "branch.main.remote", "."]
        ));
        assert!(run_git(
            directory.path(),
            &["config", "branch.main.merge", "refs/heads/main"]
        ));

        let clean = discover_repository(directory.path()).unwrap().unwrap();
        assert_eq!(clean.branch.as_deref(), Some("main"));
        assert!(clean.head_oid.as_deref().is_some_and(valid_oid));
        assert!(clean.dirty_hash.is_none());

        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let (legacy_status, legacy_truncated) = git_stdout_bounded(
            directory.path(),
            &["status", "--porcelain=v2", "-z", "--untracked-files=normal"],
            STATUS_LIMIT,
        )
        .unwrap()
        .unwrap();
        let (metadata_status, metadata_truncated) = git_stdout_bounded(
            directory.path(),
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=normal",
            ],
            STATUS_LIMIT + STATUS_METADATA_ALLOWANCE,
        )
        .unwrap()
        .unwrap();
        let (_, _, filtered_status, filtered_truncated) =
            parse_status_metadata(&metadata_status, metadata_truncated);
        assert_eq!(filtered_status, legacy_status);
        assert_eq!(filtered_truncated, legacy_truncated);

        let dirty = discover_repository(directory.path()).unwrap().unwrap();
        assert_eq!(dirty.branch, clean.branch);
        assert_eq!(dirty.head_oid, clean.head_oid);
        assert!(dirty.dirty_hash.is_some());
    }

    #[test]
    fn identical_relative_remotes_do_not_collide_across_local_repositories() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("one/repo");
        let second = directory.path().join("two/repo");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        assert!(run_git(&first, &["init", "--initial-branch=main"]));
        assert!(run_git(&second, &["init", "--initial-branch=main"]));
        assert!(run_git(
            &first,
            &["config", "remote.origin.url", "../origin.git"],
        ));
        assert!(run_git(
            &second,
            &["config", "remote.origin.url", "../origin.git"],
        ));

        let first_context = discover_repository(&first).unwrap().unwrap();
        let second_context = discover_repository(&second).unwrap().unwrap();
        assert_ne!(first_context.remote, second_context.remote);
        assert_ne!(first_context.repo_id, second_context.repo_id);
    }

    #[test]
    fn invalid_revision_is_unknown_without_running_git() {
        assert_eq!(
            compare_revisions(".", "--help", "abcdef1234567"),
            GitRelation::Unknown
        );
    }

    #[test]
    fn linked_worktrees_share_local_repository_identity() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let primary = directory.path().join("primary");
        let linked = directory.path().join("linked");
        std::fs::create_dir(&primary).unwrap();
        if !run_git(&primary, &["init", "--initial-branch=main"])
            || !run_git(&primary, &["config", "user.email", "tests@example.test"])
            || !run_git(&primary, &["config", "user.name", "Super Mem Tests"])
        {
            return;
        }
        std::fs::write(primary.join("README.md"), "initial\n").unwrap();
        assert!(run_git(&primary, &["add", "README.md"]));
        assert!(run_git(&primary, &["commit", "-m", "initial"]));
        assert!(run_git(
            &primary,
            &[
                "worktree",
                "add",
                "-b",
                "linked-test",
                linked.to_str().unwrap(),
            ],
        ));

        let primary_context = discover_repository(&primary).unwrap().unwrap();
        let linked_context = discover_repository(&linked).unwrap().unwrap();
        assert_ne!(primary_context.root, linked_context.root);
        assert_eq!(primary_context.common_dir, linked_context.common_dir);
        assert_eq!(primary_context.repo_id, linked_context.repo_id);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn non_utf8_git_roots_have_distinct_native_identities() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let first = directory
            .path()
            .join(OsString::from_vec(vec![b'r', b'e', b'p', b'o', 0xfe]));
        let second = directory
            .path()
            .join(OsString::from_vec(vec![b'r', b'e', b'p', b'o', 0xff]));
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        assert_ne!(
            canonical_path_digest(&first),
            canonical_path_digest(&second)
        );
        assert!(run_git(&first, &["init", "--initial-branch=main"]));
        assert!(run_git(&second, &["init", "--initial-branch=main"]));

        let first_context = discover_repository(&first).unwrap().unwrap();
        let second_context = discover_repository(&second).unwrap().unwrap();
        assert_ne!(first_context.repo_id, second_context.repo_id);
    }

    #[cfg(unix)]
    #[test]
    fn trailing_newline_is_preserved_in_git_root_identity() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let plain = directory.path().join("repo");
        let newline = directory
            .path()
            .join(OsString::from_vec(b"repo\n".to_vec()));
        std::fs::create_dir(&plain).unwrap();
        std::fs::create_dir(&newline).unwrap();
        assert!(run_git(&plain, &["init", "--initial-branch=main"]));
        assert!(run_git(&newline, &["init", "--initial-branch=main"]));

        let plain_context = discover_repository(&plain).unwrap().unwrap();
        let newline_context = discover_repository(&newline).unwrap().unwrap();
        assert_ne!(plain_context.repo_id, newline_context.repo_id);
        assert_ne!(plain_context.root, newline_context.root);
        assert_ne!(plain_context.common_dir, newline_context.common_dir);
    }
}
