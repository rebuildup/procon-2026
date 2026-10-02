//! Contest deliverable bundle builder.
//!
//! Produces a zip archive that bundles the release-built `procon` binary,
//! contest documentation, and `.env.example` for submission to the
//! 予選 / 本選 review process.
//!
//! The [`build_bundle`] helper is the only zip-writing primitive — it accepts
//! already-in-memory payloads. Higher-level orchestration (locating files,
//! invoking `cargo build --release`, linting for accidental secrets) lives in
//! the CLI command that calls this function.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// One entry that will be written into the bundle archive.
///
/// * `archive_path` — path inside the zip (forward-slash separators).
/// * `contents`     — raw file bytes.
pub type BundleEntry = (PathBuf, Vec<u8>);

/// Unix permissions to apply to an entry. Defaults to `0o644`.
#[derive(Debug, Clone, Copy)]
pub struct BundleEntryPerms(pub u32);

impl Default for BundleEntryPerms {
    fn default() -> Self {
        Self(0o644)
    }
}

impl BundleEntryPerms {
    /// Executable (typical for the `procon` binary on Linux).
    pub const EXECUTABLE: Self = Self(0o755);
}

/// One entry with explicit permissions (used by [`build_bundle_with_perms`]).
pub struct PermissiveEntry {
    pub path: PathBuf,
    pub contents: Vec<u8>,
    pub perms: BundleEntryPerms,
}

/// Orchestrate `procon bundle build <output>` end-to-end:
/// locate the workspace root, build the release binary, gather the
/// documented file set, lint for secrets, write the zip, and print a
/// human-readable summary.
///
/// This is the function the CLI subcommand calls. It is intentionally
/// `async` so it composes with the rest of the CLI runtime even though the
/// actual work is fully blocking — `cargo build` is the slowest step and we
/// do not want to wrap it in a thread just for symmetry.
pub async fn run_build(output_path: &Path) -> Result<()> {
    let workspace = locate_workspace_root()?;
    let binary_path = workspace.join("target").join("release").join("procon");
    ensure_release_binary(&binary_path, &workspace).await?;

    let entries = collect_entries(&workspace, &binary_path)?;
    let lint_targets: Vec<BundleEntry> =
        entries.iter().map(|e| (e.path.clone(), e.contents.clone())).collect();
    let findings = lint_secrets(&lint_targets);
    if !findings.is_empty() {
        eprintln!("secret lint findings:");
        for f in &findings {
            eprintln!("  - {f}");
        }
        bail!("aborting bundle: {} potential secret leak(s) detected", findings.len());
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir {}", parent.display()))?;
        }
    }

    let total_uncompressed = build_bundle_with_perms(output_path, &entries)?;
    let on_disk_size = std::fs::metadata(output_path)
        .with_context(|| format!("stat bundle {}", output_path.display()))?
        .len();

    println!("bundle written: {}", output_path.display());
    println!("files:");
    for entry in &entries {
        println!("  - {:<40} {} bytes", entry.path.display(), entry.contents.len());
    }
    println!(
        "totals: {} entries, {} uncompressed bytes, {} on disk",
        entries.len(),
        total_uncompressed,
        on_disk_size
    );
    Ok(())
}

/// Resolve the workspace root from the current executable's location.
///
/// `CARGO_MANIFEST_DIR` is set during compilation and points at
/// `crates/cli`. Its grand-parent is the workspace root.
fn locate_workspace_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .and_then(|p| p.parent())
        .context("locate workspace root from CARGO_MANIFEST_DIR")?;
    Ok(workspace.to_path_buf())
}

/// Build the release `procon` binary if it is missing.
///
/// `cargo build --release --bin procon --locked` is invoked as a subprocess
/// so we honour the same flags as `just build`. Existing up-to-date binaries
/// are reused to keep the bundle workflow fast on iteration.
async fn ensure_release_binary(binary_path: &Path, workspace: &Path) -> Result<()> {
    if binary_path.exists() {
        return Ok(());
    }
    let status = tokio::process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("procon")
        .arg("--locked")
        .current_dir(workspace)
        .status()
        .await
        .context("spawn cargo build --release")?;
    if !status.success() {
        bail!("cargo build --release failed (status {status})");
    }
    Ok(())
}

/// Build the in-memory entry list for the bundle.
///
/// Each entry's archive path is the path *relative to the workspace root* so
/// the zip has a clean, predictable layout. The `procon` binary entry is
/// granted executable permissions; the rest use default `0o644`. Missing
/// files are skipped with a warning rather than aborting — `.env.example`
/// and the docs may legitimately be absent in some branches.
fn collect_entries(workspace: &Path, binary_path: &Path) -> Result<Vec<PermissiveEntry>> {
    let entries: &[(&str, &str)] = &[
        ("procon", "procon"),
        ("README.md", "README.md"),
        ("docs/contest/strategy.md", "docs/contest/strategy.md"),
        ("docs/contest/team-context.md", "docs/contest/team-context.md"),
        ("docs/contest/api.md", "docs/contest/api.md"),
        ("docs/contest/openapi.md", "docs/contest/openapi.md"),
        (".env.example", ".env.example"),
    ];

    let mut result = Vec::new();
    for (archive, source) in entries {
        let src =
            if *source == "procon" { binary_path.to_path_buf() } else { workspace.join(source) };
        match std::fs::read(&src) {
            Ok(bytes) => {
                let perms = if *archive == "procon" {
                    BundleEntryPerms::EXECUTABLE
                } else {
                    BundleEntryPerms::default()
                };
                result.push(PermissiveEntry {
                    path: PathBuf::from(*archive),
                    contents: bytes,
                    perms,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("warning: skipping missing file {}", src.display());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("read bundle source {}", src.display()));
            }
        }
    }
    Ok(result)
}

/// Build a zip archive containing every entry in `files`.
///
/// The output file is created (or truncated) at `output_path`. Entries are
/// stored with deflate compression and `0o644` permissions so they look
/// reasonable on Unix systems while remaining valid Windows zips.
///
/// Returns the total **uncompressed** size of all entries.
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_bundle(output_path: &Path, files: &[BundleEntry]) -> Result<u64> {
    let permissives: Vec<PermissiveEntry> = files
        .iter()
        .map(|(path, contents)| PermissiveEntry {
            path: path.clone(),
            contents: contents.clone(),
            perms: BundleEntryPerms::default(),
        })
        .collect();
    build_bundle_with_perms(output_path, &permissives)
}

/// Build a zip archive from entries with explicit Unix permissions.
pub fn build_bundle_with_perms(output_path: &Path, files: &[PermissiveEntry]) -> Result<u64> {
    let file = File::create(output_path)
        .with_context(|| format!("create bundle file {}", output_path.display()))?;
    let mut writer = zip::ZipWriter::new(file);

    let mut total: u64 = 0;
    for entry in files {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(entry.perms.0);
        let name = entry.path.to_string_lossy().replace('\\', "/");
        writer.start_file(&name, options).with_context(|| format!("start zip entry {name}"))?;
        writer.write_all(&entry.contents).with_context(|| format!("write zip entry {name}"))?;
        total = total.saturating_add(entry.contents.len() as u64);
    }
    writer.finish().context("finalize bundle zip")?;
    Ok(total)
}

/// A suspicious string that looks like a leaked secret inside the bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    /// Archive path of the offending entry.
    pub archive_path: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// Short label for the pattern category.
    pub kind: String,
    /// Excerpt of the line that matched (trimmed).
    pub excerpt: String,
}

impl std::fmt::Display for SecretFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{} [{}] {}", self.archive_path.display(), self.line, self.kind, self.excerpt)
    }
}

/// Pattern categories we scan for. Each tuple is `(label, regex)`.
///
/// The regex is intentionally simple — we are not trying to be a full
/// secret-scanner, only catch the obvious mistakes (committing a real
/// `.env`, pasting an API key into docs, etc.).
const SECRET_PATTERNS: &[(&str, &str)] = &[
    ("api_key_assignment", r#"(?i)\b(api[_-]?key|apikey)\s*[=:]\s*['"]?[A-Za-z0-9+/=_-]{16,}"#),
    ("secret_assignment", r#"(?i)\b(secret|secret[_-]?key)\s*[=:]\s*['"]?[A-Za-z0-9+/=_-]{16,}"#),
    (
        "token_assignment",
        r#"(?i)\b(auth[_-]?token|access[_-]?token|bearer[_-]?token)\s*[=:]\s*['"]?[A-Za-z0-9+/=_-]{16,}"#,
    ),
    ("password_assignment", r#"(?i)\b(password|passwd|pwd)\s*[=:]\s*['"][^\s'"]{8,}"#),
];

/// Files that should never appear in a contest bundle, by basename.
const FORBIDDEN_BASENAMES: &[&str] = &[".env", "id_rsa", "id_ed25519", "*.pem", "*.key"];

/// Scan every entry in `files` for accidental secret leaks.
///
/// Returns one [`SecretFinding`] per offending line. Filenames that match
/// [`FORBIDDEN_BASENAMES`] are reported as `forbidden_filename` without
/// inspecting their contents.
pub fn lint_secrets(files: &[BundleEntry]) -> Vec<SecretFinding> {
    use regex::Regex;

    let compiled: Vec<(&str, Regex)> = SECRET_PATTERNS
        .iter()
        .filter_map(|(label, pat)| Regex::new(pat).ok().map(|r| (*label, r)))
        .collect();

    let mut findings = Vec::new();
    for (path, contents) in files {
        let basename =
            path.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
        let forbidden = FORBIDDEN_BASENAMES.iter().any(|g| {
            if let Some(stripped) = g.strip_prefix("*.") {
                basename.ends_with(&format!(".{stripped}"))
            } else {
                basename == *g
            }
        });
        if forbidden {
            findings.push(SecretFinding {
                archive_path: path.clone(),
                line: 0,
                kind: "forbidden_filename".to_string(),
                excerpt: basename,
            });
            continue;
        }
        let Ok(text) = std::str::from_utf8(contents) else {
            // binary entry (e.g. the procon binary) — skip
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            for (label, regex) in &compiled {
                if regex.is_match(line) {
                    findings.push(SecretFinding {
                        archive_path: path.clone(),
                        line: idx + 1,
                        kind: (*label).to_string(),
                        excerpt: line.trim().to_string(),
                    });
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::tempdir;

    fn entry(name: &str, data: &[u8]) -> BundleEntry {
        (PathBuf::from(name), data.to_vec())
    }

    #[test]
    fn build_bundle_writes_all_entries() {
        let dir = tempdir().expect("tempdir");
        let out = dir.path().join("bundle.zip");
        let files =
            vec![entry("README.md", b"# hello\n"), entry("nested/dir/data.txt", b"payload")];
        let total = build_bundle(&out, &files).expect("build_bundle");
        assert_eq!(total, 15);

        let f = File::open(&out).expect("open");
        let mut zip = zip::ZipArchive::new(f).expect("read zip");
        assert_eq!(zip.len(), 2);

        let mut names: Vec<String> =
            (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["README.md", "nested/dir/data.txt"]);

        let mut buf = String::new();
        zip.by_name("README.md").unwrap().read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "# hello\n");
    }

    #[test]
    fn build_bundle_round_trip_larger_payload() {
        let dir = tempdir().expect("tempdir");
        let out = dir.path().join("bundle.zip");
        let payload: Vec<u8> = (0u8..=255).cycle().take(8192).collect();
        let files = vec![entry("blob.bin", &payload)];
        let total = build_bundle(&out, &files).expect("build_bundle");
        assert_eq!(total, 8192);

        let f = File::open(&out).expect("open");
        let mut zip = zip::ZipArchive::new(f).expect("read zip");
        let mut out_buf = Vec::new();
        zip.by_name("blob.bin").unwrap().read_to_end(&mut out_buf).unwrap();
        assert_eq!(out_buf, payload);
    }

    #[test]
    fn build_bundle_replaces_existing_file() {
        let dir = tempdir().expect("tempdir");
        let out = dir.path().join("bundle.zip");
        build_bundle(&out, &[entry("a", b"first")]).unwrap();
        build_bundle(&out, &[entry("b", b"second")]).unwrap();
        let f = File::open(&out).unwrap();
        let mut zip = zip::ZipArchive::new(f).unwrap();
        assert_eq!(zip.len(), 1);
        let mut buf = Vec::new();
        zip.by_name("b").unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"second");
    }

    #[test]
    fn lint_detects_forbidden_filename() {
        let files = vec![entry("secrets/.env", b"API_KEY=abcd1234abcd1234")];
        let findings = lint_secrets(&files);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, "forbidden_filename");
        assert_eq!(findings[0].archive_path, PathBuf::from("secrets/.env"));
    }

    #[test]
    fn lint_detects_api_key_assignment() {
        let files = vec![entry("docs/notes.md", b"my key is api_key = abcdefghijklmnop1234\n")];
        let findings = lint_secrets(&files);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, "api_key_assignment");
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn lint_ignores_env_example_placeholders() {
        let files = vec![entry(
            ".env.example",
            b"PROCON_ENGINE_PORT=7878\nVITE_API_BASE_URL=http://localhost:7878\n",
        )];
        let findings = lint_secrets(&files);
        assert!(findings.is_empty(), "got {findings:?}");
    }

    #[test]
    fn lint_skips_binary_entries() {
        // Non-UTF8 bytes — must not panic and must not flag anything.
        let files = vec![entry("procon", &[0xff, 0xfe, 0x00, 0x01, 0x80])];
        let findings = lint_secrets(&files);
        assert!(findings.is_empty());
    }
}
