use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    All,
    User,
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

#[derive(Debug)]
pub struct DotConf {
    backup_directory: PathBuf,
    symlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
    sys_symlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl DotConf {
    pub fn from_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        let config_path = resolve_from_cwd(path.as_ref())?;
        let yaml = fs::read_to_string(&config_path)
            .with_context(|| format!("failed reading {}", config_path.display()))?;

        let base_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
        Self::from_yaml_str(&yaml, base_dir)
            .with_context(|| format!("failed parsing {}", config_path.display()))
    }

    pub fn from_yaml_str(yaml: &str, base_dir: &Path) -> Result<Self> {
        let raw: RawConfig = serde_yml::from_str(yaml)?;
        let cwd = std::env::current_dir().context("failed reading current directory")?;

        Ok(Self {
            backup_directory: resolve_against(&cwd, &raw.backup_directory),
            symlinks: normalize_links(base_dir, &cwd, raw.symlinks),
            sys_symlinks: normalize_links(base_dir, &cwd, raw.sys_symlinks),
        })
    }

    pub fn requires_root(&self) -> bool {
        !self.sys_symlinks.is_empty()
    }

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
    base_dir: &Path,
    cwd: &Path,
    links: BTreeMap<PathBuf, OneOrMany>,
) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    links
        .into_iter()
        .map(|(source, dests)| {
            let source = resolve_against(base_dir, &source);
            let dests = dests
                .into_vec()
                .into_iter()
                .map(|p| resolve_against(cwd, &p))
                .collect();
            (source, dests)
        })
        .collect()
}

fn resolve_from_cwd(path: &Path) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed reading current directory")?;
    Ok(resolve_against(&cwd, path))
}

fn resolve_against(base: &Path, path: &Path) -> PathBuf {
    let expanded = expand_tilde(path);
    if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    }
}

fn expand_tilde(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let Some(home) = home::home_dir() else {
        return path.to_path_buf();
    };

    if raw == "~" {
        return home;
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    path.to_path_buf()
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
        create_symlink(&target, &backup)?;
        remove_symlink(destination)?;
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

    for attempt in 0..1000 {
        let backup = backup_directory.join(format!("{name}.{hash:016x}.{ts}.{attempt}.bak"));
        match fs::symlink_metadata(&backup) {
            Ok(_) => continue,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(backup),
            Err(err) => {
                return Err(err).with_context(|| format!("failed inspecting {}", backup.display()));
            }
        }
    }

    bail!(
        "failed finding unique backup path for {}",
        destination.display()
    )
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

fn remove_symlink(path: &Path) -> Result<()> {
    symlink::remove_symlink_auto(path)
        .with_context(|| format!("failed removing {}", path.display()))
}
