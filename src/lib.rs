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
    pub config_path: PathBuf,
    pub backup_directory: PathBuf,
    pub symlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
    pub sys_symlinks: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl DotConf {
    pub fn from_yaml_file(path: impl AsRef<Path>) -> Result<Self> {
        let config_path = absolutize(path.as_ref(), None);
        let yaml = fs::read_to_string(&config_path)
            .with_context(|| format!("failed reading {}", config_path.display()))?;
        let raw: RawConfig = serde_yaml::from_str(&yaml)
            .with_context(|| format!("failed parsing {}", config_path.display()))?;

        let base_dir = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        Ok(Self {
            config_path,
            backup_directory: absolutize(&raw.backup_directory, None),
            symlinks: normalize_links(&base_dir, raw.symlinks),
            sys_symlinks: normalize_links(&base_dir, raw.sys_symlinks),
        })
    }

    pub fn requires_root(&self) -> bool {
        !self.sys_symlinks.is_empty()
    }

    pub fn apply(&self, scope: Scope) -> Result<()> {
        fs::create_dir_all(&self.backup_directory)
            .with_context(|| format!("failed creating {}", self.backup_directory.display()))?;

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
            let source_metadata = match fs::metadata(source) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
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
                create_symlink(source, destination, source_metadata.is_dir())?;
            }
        }
        Ok(())
    }
}

fn normalize_links(
    base_dir: &Path,
    links: BTreeMap<PathBuf, OneOrMany>,
) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    links
        .into_iter()
        .map(|(source, dests)| {
            let source = absolutize(source, Some(base_dir));
            let dests = dests
                .into_vec()
                .into_iter()
                .map(|p| absolutize(p, None))
                .collect();
            (source, dests)
        })
        .collect()
}

fn absolutize(path: impl AsRef<Path>, relative_to: Option<&Path>) -> PathBuf {
    let path = path.as_ref();
    let with_tilde = expand_home(path);

    if with_tilde.is_absolute() {
        with_tilde
    } else if let Some(base) = relative_to {
        base.join(with_tilde)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(with_tilde)
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return home_dir().unwrap_or_else(|_| path.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|_| path.to_path_buf());
    }
    path.to_path_buf()
}

fn home_dir() -> std::result::Result<PathBuf, std::env::VarError> {
    #[cfg(windows)]
    {
        if let Some(home) = std::env::var_os("USERPROFILE") {
            return Ok(PathBuf::from(home));
        }
        if let (Some(mut drive), Some(path)) =
            (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH"))
        {
            drive.push(path);
            return Ok(PathBuf::from(drive));
        }
    }

    std::env::var("HOME").map(PathBuf::from)
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
        let target = fs::read_link(destination)?;
        let target_is_dir = fs::metadata(destination)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        create_symlink(&target, &backup, target_is_dir)?;
        remove_symlink(destination, target_is_dir)?;
        return Ok(());
    }

    if metadata.is_file() {
        fs::copy(destination, &backup)?;
        fs::remove_file(destination)?;
        return Ok(());
    }

    bail!(
        "destination {} exists and is not a file/symlink",
        destination.display()
    )
}

fn unique_backup_path(backup_directory: &Path, destination: &Path) -> Result<PathBuf> {
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

fn create_symlink(source: &Path, destination: &Path, source_is_dir: bool) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = source_is_dir;
        std::os::unix::fs::symlink(source, destination).with_context(|| {
            format!(
                "failed to create symlink {} -> {}",
                destination.display(),
                source.display()
            )
        })?;
    }

    #[cfg(windows)]
    {
        let symlink = if source_is_dir {
            std::os::windows::fs::symlink_dir
        } else {
            std::os::windows::fs::symlink_file
        };
        symlink(source, destination).with_context(|| {
            format!(
                "failed to create symlink {} -> {}",
                destination.display(),
                source.display()
            )
        })?;
    }

    Ok(())
}

fn remove_symlink(path: &Path, target_is_dir: bool) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = target_is_dir;
        fs::remove_file(path).with_context(|| format!("failed removing {}", path.display()))?;
    }

    #[cfg(windows)]
    {
        let remove = if target_is_dir {
            fs::remove_dir
        } else {
            fs::remove_file
        };
        remove(path).with_context(|| format!("failed removing {}", path.display()))?;
    }

    Ok(())
}
