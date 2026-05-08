use std::collections::BTreeMap;
use std::fs;
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
                self.apply_links(&self.symlinks)?;
                self.apply_links(&self.sys_symlinks)?;
            }
            Scope::User => self.apply_links(&self.symlinks)?,
            Scope::Sys => self.apply_links(&self.sys_symlinks)?,
        }
        Ok(())
    }

    fn apply_links(&self, links: &BTreeMap<PathBuf, Vec<PathBuf>>) -> Result<()> {
        for (source, destinations) in links {
            if !source.exists() {
                continue;
            }
            for destination in destinations {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
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
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| path.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|_| path.to_path_buf());
    }
    path.to_path_buf()
}

fn backup_and_remove_if_exists(backup_directory: &Path, destination: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(()),
    };

    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let backup = backup_directory.join(format!("{name}.{ts}.bak"));

    if metadata.file_type().is_symlink() {
        let target = fs::read_link(destination)?;
        create_symlink(&target, &backup)?;
        fs::remove_file(destination)?;
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

fn create_symlink(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            destination.display(),
            source.display()
        )
    })?;

    #[cfg(windows)]
    std::os::windows::fs::symlink_file(source, destination).with_context(|| {
        format!(
            "failed to create symlink {} -> {}",
            destination.display(),
            source.display()
        )
    })?;

    Ok(())
}
