#![deny(missing_docs)]
//! Apply dotfile symlink configurations described by YAML.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Which configured links to apply.
///
/// [`Scope::All`] applies system links first, so a system-link failure aborts
/// before user state is touched. That ordering is part of the failure behavior
/// covered by integration tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    /// Apply system links, then user links.
    All,
    /// Apply user links only.
    User,
    /// Apply system links only.
    Sys,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    backup_directory: PathBuf,
    #[serde(default)]
    symlinks: BTreeMap<PathBuf, OneOrMany>,
    #[serde(default)]
    sys_symlinks: BTreeMap<PathBuf, OneOrMany>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(PathBuf),
    Many(Vec<PathBuf>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<PathBuf> {
        match self {
            Self::One(path) => vec![path],
            Self::Many(paths) => paths,
        }
    }
}

/// Parsed dot-conf configuration ready to apply.
#[derive(Debug)]
pub struct DotConf {
    backup_directory: PathBuf,
    symlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
    sys_symlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl DotConf {
    /// Load a configuration from a YAML file.
    ///
    /// Relative source paths and `backup_directory` are resolved against the
    /// canonical directory containing the YAML file. Relative destinations are
    /// resolved against the process current working directory.
    pub fn from_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        let config_path = resolve_from_cwd(path.as_ref())?;
        let config_path = config_path
            .canonicalize()
            .with_context(|| format!("failed resolving {}", config_path.display()))?;
        let yaml = fs::read_to_string(&config_path)
            .with_context(|| format!("failed reading {}", config_path.display()))?;
        let cwd = std::env::current_dir().context("failed reading current directory")?;

        let source_base = config_path.parent().unwrap_or_else(|| Path::new("."));
        Self::from_yaml_str(&yaml, source_base, &cwd)
            .with_context(|| format!("in {}", config_path.display()))
    }

    /// Parse a configuration from YAML with explicit path bases.
    ///
    /// `source_base` is used for relative source paths and
    /// `backup_directory`. `destination_base` is used for relative
    /// destinations. Supplying both bases keeps parsing independent of process
    /// current directory.
    pub fn from_yaml_str(yaml: &str, source_base: &Path, destination_base: &Path) -> Result<Self> {
        let raw: RawConfig = serde_yml::from_str(yaml).context("failed parsing YAML")?;
        Ok(Self {
            backup_directory: resolve_against(source_base, &raw.backup_directory)
                .with_context(|| format!("failed resolving {}", raw.backup_directory.display()))?,
            symlinks: normalize_links(source_base, destination_base, raw.symlinks)?,
            sys_symlinks: normalize_links(source_base, destination_base, raw.sys_symlinks)?,
        })
    }

    /// Return whether this configuration includes system links.
    pub fn requires_root(&self) -> bool {
        !self.sys_symlinks.is_empty()
    }

    /// Apply configured symlinks for the requested scope.
    ///
    /// Existing file and symlink destinations are backed up before they are
    /// replaced. Missing source files are skipped with a warning.
    pub fn apply(&self, scope: Scope) -> Result<()> {
        match scope {
            Scope::All => {
                self.apply_links(&self.sys_symlinks)?;
                self.apply_links(&self.symlinks)?;
            }
            Scope::User => self.apply_links(&self.symlinks)?,
            Scope::Sys => self.apply_links(&self.sys_symlinks)?,
        }
        Ok(())
    }

    fn apply_links(&self, links: &BTreeMap<PathBuf, Vec<PathBuf>>) -> Result<()> {
        for (source, destinations) in links {
            match fs::metadata(source) {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    log::warn!("skipping missing source {}", source.display());
                    continue;
                }
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed inspecting {}", source.display()));
                }
            };

            for destination in destinations {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("failed creating {}", parent.display()))?;
                }
                backup_and_remove_if_exists(&self.backup_directory, destination)?;
                create_symlink(source, destination)?;
            }
        }
        Ok(())
    }
}

fn normalize_links(
    source_base: &Path,
    destination_base: &Path,
    links: BTreeMap<PathBuf, OneOrMany>,
) -> Result<BTreeMap<PathBuf, Vec<PathBuf>>> {
    let mut normalized = BTreeMap::new();
    for (source, destinations) in links {
        let resolved_source = resolve_against(source_base, &source)
            .with_context(|| format!("failed resolving source {}", source.display()))?;
        let resolved_destinations = destinations
            .into_vec()
            .into_iter()
            .map(|destination| {
                resolve_against(destination_base, &destination).with_context(|| {
                    format!("failed resolving destination {}", destination.display())
                })
            })
            .collect::<Result<Vec<_>>>()?;
        normalized.insert(resolved_source, resolved_destinations);
    }
    Ok(normalized)
}

fn resolve_from_cwd(path: &Path) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed reading current directory")?;
    resolve_against(&cwd, path)
}

fn resolve_against(base: &Path, path: &Path) -> Result<PathBuf> {
    let expanded = expand_tilde(path)?;
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    })
}

#[cfg(unix)]
fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let home = home::home_dir();
    expand_tilde_with_home(path, home.as_deref())
}

#[cfg(unix)]
fn expand_tilde_with_home(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    if bytes == b"~" {
        return Ok(home.map_or_else(|| path.to_path_buf(), Path::to_path_buf));
    }
    if let Some(rest) = bytes.strip_prefix(b"~/") {
        return Ok(home.map_or_else(
            || path.to_path_buf(),
            |home| home.join(OsStr::from_bytes(rest)),
        ));
    }
    if bytes.starts_with(b"~") {
        bail!(
            "unsupported home path {}; use ~ or ~/ for the current user",
            path.display()
        );
    }
    Ok(path.to_path_buf())
}

#[cfg(windows)]
fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let home = home::home_dir();
    expand_tilde_with_home(path, home.as_deref())
}

#[cfg(windows)]
fn expand_tilde_with_home(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let raw = path.as_os_str().to_string_lossy();
    if raw == "~" {
        return Ok(home.map_or_else(|| path.to_path_buf(), Path::to_path_buf));
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return Ok(home.map_or_else(|| path.to_path_buf(), |home| home.join(Path::new(rest))));
    }
    if raw.starts_with('~') {
        bail!(
            "unsupported home path {}; use ~, ~/, or ~\\ for the current user",
            path.display()
        );
    }
    Ok(path.to_path_buf())
}

#[cfg(not(any(unix, windows)))]
fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let raw = path.to_string_lossy();
    let Some(home) = home::home_dir() else {
        return Ok(path.to_path_buf());
    };

    if raw == "~" {
        return Ok(home);
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    if raw.starts_with('~') {
        bail!(
            "unsupported home path {}; use ~ or ~/ for the current user",
            path.display()
        );
    }
    Ok(path.to_path_buf())
}

fn backup_and_remove_if_exists(backup_directory: &Path, destination: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed inspecting {}", destination.display()));
        }
    };

    let backup = unique_backup_path(backup_directory, destination)?;

    if metadata.file_type().is_symlink() {
        let target = symlink_backup_target(destination)?;
        let kind = symlink_kind(&metadata);
        create_symlink_with_kind(&target, &backup, kind)?;
        remove_symlink(destination, kind)?;
        return Ok(());
    }

    if metadata.is_file() {
        move_or_copy_file(destination, &backup)?;
        return Ok(());
    }

    bail!(
        "destination {} exists and is not a file/symlink",
        destination.display()
    )
}

fn unique_backup_path(backup_directory: &Path, destination: &Path) -> Result<PathBuf> {
    fs::create_dir_all(backup_directory)
        .with_context(|| format!("failed creating {}", backup_directory.display()))?;

    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let name = backup_name(destination);
    let hash = path_hash(destination);
    let backup = backup_directory.join(format!("{name}.{hash:016x}.{ts}.bak"));

    match fs::symlink_metadata(&backup) {
        Ok(_) => bail!("backup path {} already exists", backup.display()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(backup),
        Err(err) => Err(err).with_context(|| format!("failed inspecting {}", backup.display())),
    }
}

fn symlink_backup_target(destination: &Path) -> Result<PathBuf> {
    let target = fs::read_link(destination)
        .with_context(|| format!("failed reading symlink {}", destination.display()))?;
    if target.is_absolute() {
        return Ok(target);
    }

    Ok(destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(target))
}

fn backup_name(destination: &Path) -> String {
    let raw = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup");
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if sanitized.is_empty() {
        "backup".to_string()
    } else {
        sanitized
    }
}

fn path_hash(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn move_or_copy_file(source: &Path, destination: &Path) -> Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::CrossesDevices => {
            fs::copy(source, destination).with_context(|| {
                format!(
                    "failed copying {} -> {}",
                    source.display(),
                    destination.display()
                )
            })?;
            fs::remove_file(source)
                .with_context(|| format!("failed removing {}", source.display()))?;
            Ok(())
        }
        Err(err) => Err(err).with_context(|| format!("failed moving {}", source.display())),
    }
}

fn create_symlink(source: &Path, destination: &Path) -> Result<()> {
    symlink::symlink_auto(source, destination).with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            destination.display(),
            source.display()
        )
    })
}

#[derive(Clone, Copy)]
enum SymlinkKind {
    File,
    #[cfg(windows)]
    Dir,
}

#[cfg(windows)]
fn symlink_kind(metadata: &fs::Metadata) -> SymlinkKind {
    use std::os::windows::fs::FileTypeExt;

    if metadata.file_type().is_symlink_dir() {
        SymlinkKind::Dir
    } else {
        SymlinkKind::File
    }
}

#[cfg(not(windows))]
fn symlink_kind(_metadata: &fs::Metadata) -> SymlinkKind {
    SymlinkKind::File
}

fn create_symlink_with_kind(source: &Path, destination: &Path, kind: SymlinkKind) -> Result<()> {
    let result = match kind {
        SymlinkKind::File => symlink::symlink_file(source, destination),
        #[cfg(windows)]
        SymlinkKind::Dir => symlink::symlink_dir(source, destination),
    };
    result.with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            destination.display(),
            source.display()
        )
    })
}

fn remove_symlink(path: &Path, kind: SymlinkKind) -> Result<()> {
    // The existing link metadata carries the Windows file/dir distinction even
    // for dangling links, so reuse it instead of probing the target.
    let result = match kind {
        SymlinkKind::File => symlink::remove_symlink_file(path),
        #[cfg(windows)]
        SymlinkKind::Dir => symlink::remove_symlink_dir(path),
    };
    result.with_context(|| format!("failed removing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn expand_tilde_preserves_non_utf8_suffix() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = PathBuf::from(OsString::from_vec(b"~/foo\xffbar".to_vec()));
        let expanded = expand_tilde_with_home(&path, Some(Path::new("/tmp/home"))).unwrap();

        assert_eq!(expanded.as_os_str().as_bytes(), b"/tmp/home/foo\xffbar");
    }

    #[cfg(windows)]
    #[test]
    fn expand_tilde_accepts_windows_separator() {
        let expanded =
            expand_tilde_with_home(Path::new("~\\AppData"), Some(Path::new("C:\\Users\\ben")))
                .unwrap();

        assert_eq!(expanded, PathBuf::from("C:\\Users\\ben\\AppData"));
    }
}
