# Installation

## Download a binary

Open the [latest release](https://github.com/Limme-swe/super-mem/releases/latest) and download the archive matching your system:

| System | Archive suffix |
| --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-musl.tar.gz` |
| Windows x86-64 | `x86_64-pc-windows-msvc.zip` |
| macOS Apple Silicon | `aarch64-apple-darwin.tar.gz` |
| macOS Intel | `x86_64-apple-darwin.tar.gz` |

The Linux binary is statically linked. The Windows binary uses the static MSVC runtime. The macOS binaries target macOS 11 or newer.

### Linux

~~~sh
tar -xzf super-mem-v0.1.0-x86_64-unknown-linux-musl.tar.gz
install -Dm755 super-mem-v0.1.0-x86_64-unknown-linux-musl/supermem "$HOME/.local/bin/supermem"
"$HOME/.local/bin/supermem" --version
~~~

Add `$HOME/.local/bin` to `PATH` if your shell does not already include it.

### macOS

Choose the archive for the current processor and install it:

~~~sh
case "$(uname -m)" in
  arm64) TARGET=aarch64-apple-darwin ;;
  x86_64) TARGET=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac
ARCHIVE="super-mem-v0.1.0-$TARGET.tar.gz"
tar -xzf "$ARCHIVE"
mkdir -p "$HOME/.local/bin"
install -m755 "super-mem-v0.1.0-$TARGET/supermem" "$HOME/.local/bin/supermem"
"$HOME/.local/bin/supermem" --version
~~~

The release is not signed with an Apple Developer ID or notarized. macOS may block the first launch. Verify the checksum and provenance, then use **System Settings → Privacy & Security → Open Anyway** if you trust the binary. The Intel archive uses `x86_64-apple-darwin` in place of `aarch64-apple-darwin`.

### Windows

In PowerShell, extract the archive and install it under your user profile:

~~~powershell
Expand-Archive .\super-mem-v0.1.0-x86_64-pc-windows-msvc.zip -DestinationPath .
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\super-mem\bin'
New-Item -ItemType Directory -Force $InstallDir | Out-Null
Copy-Item .\super-mem-v0.1.0-x86_64-pc-windows-msvc\supermem.exe $InstallDir
$env:Path = "$InstallDir;$env:Path"
$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($UserPath -split ';') -notcontains $InstallDir) {
  [Environment]::SetEnvironmentVariable('Path', "$InstallDir;$UserPath", 'User')
}
& "$InstallDir\supermem.exe" --version
~~~

The command updates the current shell and persists the directory in the user `PATH` for future shells. The executable is not Authenticode-signed, so SmartScreen may show a warning. Verify the checksum and provenance before approving it. Native file-identity checks reject multiply linked database files without requiring administrator privileges.

## Verify a release

Download `SHA256SUMS` alongside the archive.

Linux:

~~~sh
ARCHIVE=super-mem-v0.1.0-x86_64-unknown-linux-musl.tar.gz
grep "  $ARCHIVE$" SHA256SUMS | sha256sum --check -
~~~

macOS:

~~~sh
case "$(uname -m)" in
  arm64) TARGET=aarch64-apple-darwin ;;
  x86_64) TARGET=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac
ARCHIVE="super-mem-v0.1.0-$TARGET.tar.gz"
grep "  $ARCHIVE$" SHA256SUMS | shasum -a 256 --check -
~~~

Windows PowerShell:

~~~powershell
$Archive = '.\super-mem-v0.1.0-x86_64-pc-windows-msvc.zip'
$Expected = (Select-String -Path .\SHA256SUMS -Pattern ([regex]::Escape((Split-Path $Archive -Leaf)))).Line.Split(' ')[0]
if ((Get-FileHash $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Expected) { throw 'Checksum mismatch' }
~~~

GitHub provenance can also be checked with the GitHub CLI:

~~~sh
gh attestation verify "$ARCHIVE" --repo Limme-swe/super-mem
~~~

In PowerShell, use `gh attestation verify $Archive --repo Limme-swe/super-mem`.

## Default data location

| System | Database |
| --- | --- |
| Linux | `$XDG_DATA_HOME/super-mem/memory.sqlite3`, or `~/.local/share/super-mem/memory.sqlite3` |
| macOS | `~/Library/Application Support/super-mem/memory.sqlite3` |
| Windows | `%LOCALAPPDATA%\super-mem\memory.sqlite3` |

`--db` or `SUPER_MEM_DB` overrides the default on every platform.

On Windows, keep the database on a local NTFS or ReFS volume. Network shares are unsupported because their SQLite locking and file-identity metadata can differ from local filesystems.

Git is optional for unscoped memory operations. Keep `git` on `PATH` to enable repository identity, commit ancestry, dirty-worktree classification, changed-file capture, and artifact freshness—the coding-specific applicability checks that prevent stale memories from being treated as current.

## Build from source

The workspace requires Rust 1.88 or newer:

~~~sh
git clone https://github.com/Limme-swe/super-mem.git
cd super-mem
cargo install --path crates/super-mem-cli --locked
~~~
