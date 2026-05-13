#![deny(missing_docs)]
//! Apply dotfile symlink configurations described by YAML.

use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

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

/// Which section a link came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkScope {
    /// A link from the `symlinks` section.
    User,
    /// A link from the `sys_symlinks` section.
    System,
}

/// A non-mutating preview of a configured link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkPreview {
    /// Whether the link came from `symlinks` or `sys_symlinks`.
    pub scope: LinkScope,
    /// The resolved source path.
    pub source: PathBuf,
    /// The resolved destination path.
    pub destination: PathBuf,
    /// What applying this link would do.
    pub state: LinkPreviewState,
}

/// The result of inspecting one configured link without applying it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkPreviewState {
    /// The source path is missing, so the link would be skipped.
    MissingSource,
    /// The destination is absent, so a new symlink would be created.
    Create,
    /// The destination is a file and would be backed up before replacement.
    ReplaceFile {
        /// The directory where the existing file would be backed up.
        backup_directory: PathBuf,
    },
    /// The destination is a symlink and would be backed up before replacement.
    ReplaceSymlink {
        /// The directory where the existing symlink would be backed up.
        backup_directory: PathBuf,
        /// The resolved target of the existing symlink.
        target: PathBuf,
    },
    /// The destination exists but cannot be replaced by this tool.
    Blocked {
        /// A human-readable reason the link cannot be applied.
        reason: String,
    },
    /// The link needs elevated privileges before it can be fully validated.
    NeedsElevation {
        /// A human-readable reason elevated validation is needed.
        reason: String,
    },
}

/// Options controlling how non-mutating previews inspect links.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewOptions {
    /// Treat permission-denied system-link probes as needing elevated
    /// validation instead of hard blockers.
    pub system_links_may_use_elevation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    backup_directory: PathBuf,
    #[serde(default)]
    symlinks: BTreeMap<PathBuf, RawLink>,
    #[serde(default)]
    sys_symlinks: BTreeMap<PathBuf, RawLink>,
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrManyString {
    One(String),
    Many(Vec<String>),
}

impl Default for OneOrManyString {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl OneOrManyString {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawLink {
    Destinations(OneOrMany),
    Options(RawLinkOptions),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLinkOptions {
    #[serde(alias = "destination")]
    destinations: OneOrMany,
    #[serde(default, alias = "host")]
    hosts: OneOrManyString,
}

impl RawLink {
    fn into_parts(self) -> (OneOrMany, Vec<String>) {
        match self {
            Self::Destinations(destinations) => (destinations, Vec::new()),
            Self::Options(options) => (options.destinations, options.hosts.into_vec()),
        }
    }
}

/// Parsed dot-conf configuration ready to apply.
#[derive(Debug)]
pub struct DotConf {
    backup_directory: PathBuf,
    symlinks: BTreeMap<PathBuf, LinkConfig>,
    sys_symlinks: BTreeMap<PathBuf, LinkConfig>,
}

#[derive(Debug)]
struct LinkConfig {
    destinations: Vec<PathBuf>,
    hosts: Vec<String>,
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

    /// Inspect what applying the requested scope would do without changing files.
    pub fn preview(&self, scope: Scope) -> Result<Vec<LinkPreview>> {
        self.preview_with_options(scope, PreviewOptions::default())
    }

    /// Inspect what applying the requested scope would do using preview options.
    pub fn preview_with_options(
        &self,
        scope: Scope,
        options: PreviewOptions,
    ) -> Result<Vec<LinkPreview>> {
        let mut previews = Vec::new();
        let mut current_host = None;
        match scope {
            Scope::All => {
                self.preview_links(
                    LinkScope::System,
                    &self.sys_symlinks,
                    &mut previews,
                    options,
                    &mut current_host,
                )?;
                self.preview_links(
                    LinkScope::User,
                    &self.symlinks,
                    &mut previews,
                    options,
                    &mut current_host,
                )?;
            }
            Scope::User => self.preview_links(
                LinkScope::User,
                &self.symlinks,
                &mut previews,
                options,
                &mut current_host,
            )?,
            Scope::Sys => self.preview_links(
                LinkScope::System,
                &self.sys_symlinks,
                &mut previews,
                options,
                &mut current_host,
            )?,
        }
        Ok(previews)
    }

    /// Apply configured symlinks for the requested scope.
    ///
    /// Existing file and symlink destinations are backed up before they are
    /// replaced. Missing source files are skipped with a warning.
    pub fn apply(&self, scope: Scope) -> Result<()> {
        let mut current_host = None;
        match scope {
            Scope::All => {
                self.apply_links(&self.sys_symlinks, &mut current_host)?;
                self.apply_links(&self.symlinks, &mut current_host)?;
            }
            Scope::User => self.apply_links(&self.symlinks, &mut current_host)?,
            Scope::Sys => self.apply_links(&self.sys_symlinks, &mut current_host)?,
        }
        Ok(())
    }

    fn apply_links(
        &self,
        links: &BTreeMap<PathBuf, LinkConfig>,
        current_host: &mut Option<String>,
    ) -> Result<()> {
        for (source, link) in links {
            if !link.hosts.is_empty() {
                if current_host.is_none() {
                    *current_host = Some(current_hostname()?);
                }
                let hostname = current_host.as_deref().expect("hostname was initialized");
                if !host_matches(&link.hosts, hostname) {
                    continue;
                }
            }

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

            for destination in &link.destinations {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("failed creating {}", parent.display()))?;
                }
                ensure_source_and_destination_differ(source, destination)?;
                backup_and_remove_if_exists(&self.backup_directory, destination)?;
                create_symlink(source, destination)?;
            }
        }
        Ok(())
    }

    fn preview_links(
        &self,
        scope: LinkScope,
        links: &BTreeMap<PathBuf, LinkConfig>,
        previews: &mut Vec<LinkPreview>,
        options: PreviewOptions,
        current_host: &mut Option<String>,
    ) -> Result<()> {
        for (source, link) in links {
            if !link.hosts.is_empty() {
                if current_host.is_none() {
                    *current_host = Some(current_hostname()?);
                }
                let hostname = current_host.as_deref().expect("hostname was initialized");
                if !host_matches(&link.hosts, hostname) {
                    continue;
                }
            }

            let source_exists = match fs::metadata(source) {
                Ok(_) => true,
                Err(err) if err.kind() == ErrorKind::NotFound => false,
                Err(err) if should_defer_to_elevation(scope, options, err.kind()) => {
                    for destination in &link.destinations {
                        previews.push(LinkPreview {
                            scope,
                            source: source.clone(),
                            destination: destination.clone(),
                            state: LinkPreviewState::NeedsElevation {
                                reason: format!(
                                    "failed inspecting source {} without elevated privileges: {err}",
                                    source.display()
                                ),
                            },
                        });
                    }
                    continue;
                }
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed inspecting {}", source.display()));
                }
            };

            for destination in &link.destinations {
                let state = if source_exists {
                    preview_destination(&self.backup_directory, destination, scope, options)?
                } else {
                    LinkPreviewState::MissingSource
                };
                previews.push(LinkPreview {
                    scope,
                    source: source.clone(),
                    destination: destination.clone(),
                    state,
                });
            }
        }
        Ok(())
    }
}

fn normalize_links(
    source_base: &Path,
    destination_base: &Path,
    links: BTreeMap<PathBuf, RawLink>,
) -> Result<BTreeMap<PathBuf, LinkConfig>> {
    let mut normalized = BTreeMap::new();
    for (source, raw_link) in links {
        let (destinations, hosts) = raw_link.into_parts();
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
        normalized.insert(
            resolved_source,
            LinkConfig {
                destinations: resolved_destinations,
                hosts,
            },
        );
    }
    Ok(normalized)
}

fn ensure_source_and_destination_differ(source: &Path, destination: &Path) -> Result<()> {
    if source == destination {
        bail!(
            "source and destination are the same path: {}",
            source.display()
        );
    }

    let destination_metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed inspecting {}", destination.display()));
        }
    };

    if destination_metadata.file_type().is_symlink() {
        return Ok(());
    }

    let source = source
        .canonicalize()
        .with_context(|| format!("failed resolving {}", source.display()))?;
    let destination = destination
        .canonicalize()
        .with_context(|| format!("failed resolving {}", destination.display()))?;

    if source == destination {
        bail!(
            "source and destination resolve to the same path: {}",
            source.display()
        );
    }

    Ok(())
}

fn current_hostname() -> Result<String> {
    hostname::get()
        .context("failed reading hostname")?
        .into_string()
        .map_err(|hostname| {
            anyhow!(
                "hostname is not valid UTF-8: {}",
                hostname.to_string_lossy()
            )
        })
}

fn host_matches(hosts: &[String], current_host: &str) -> bool {
    let (current_short, current_is_fqdn) = hostname_parts(current_host);

    hosts.iter().any(|host| {
        let (host_short, host_is_fqdn) = hostname_parts(host);
        host.eq_ignore_ascii_case(current_host)
            || (!host_is_fqdn && host.eq_ignore_ascii_case(current_short))
            || (!current_is_fqdn && host_short.eq_ignore_ascii_case(current_host))
    })
}

fn hostname_parts(hostname: &str) -> (&str, bool) {
    hostname
        .split_once('.')
        .map_or((hostname, false), |(short, _)| (short, true))
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

fn preview_destination(
    backup_directory: &Path,
    destination: &Path,
    scope: LinkScope,
    options: PreviewOptions,
) -> Result<LinkPreviewState> {
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            if let Some(problem) = validate_destination_parent(destination) {
                return Ok(problem.into_preview_state(scope, options));
            }
            return Ok(LinkPreviewState::Create);
        }
        Err(err) if err.kind() == ErrorKind::NotADirectory => {
            if let Some(problem) = validate_destination_parent(destination) {
                return Ok(problem.into_preview_state(scope, options));
            }
            return Ok(LinkPreviewState::Blocked {
                reason: format!("failed inspecting {}: {err}", destination.display()),
            });
        }
        Err(err) if should_defer_to_elevation(scope, options, err.kind()) => {
            return Ok(LinkPreviewState::NeedsElevation {
                reason: format!(
                    "failed inspecting destination {} without elevated privileges: {err}",
                    destination.display()
                ),
            });
        }
        Err(err) => {
            return Ok(LinkPreviewState::Blocked {
                reason: format!("failed inspecting {}: {err}", destination.display()),
            });
        }
    };

    if metadata.file_type().is_symlink() {
        let target = symlink_backup_target(destination)?;
        if let Some(problem) = validate_replacement_paths(backup_directory, destination, &metadata)
        {
            return Ok(problem.into_preview_state(scope, options));
        }
        return Ok(LinkPreviewState::ReplaceSymlink {
            backup_directory: backup_directory.to_path_buf(),
            target,
        });
    }

    if metadata.is_file() {
        if let Some(problem) = validate_replacement_paths(backup_directory, destination, &metadata)
        {
            return Ok(problem.into_preview_state(scope, options));
        }
        return Ok(LinkPreviewState::ReplaceFile {
            backup_directory: backup_directory.to_path_buf(),
        });
    }

    Ok(LinkPreviewState::Blocked {
        reason: "destination exists and is not a file/symlink".to_string(),
    })
}

#[derive(Debug)]
struct PreviewProblem {
    reason: String,
    elevation_may_fix: bool,
}

impl PreviewProblem {
    fn blocked(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            elevation_may_fix: false,
        }
    }

    fn permission_denied(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            elevation_may_fix: true,
        }
    }

    fn into_preview_state(self, scope: LinkScope, options: PreviewOptions) -> LinkPreviewState {
        if should_defer_problem_to_elevation(scope, options, self.elevation_may_fix) {
            LinkPreviewState::NeedsElevation {
                reason: self.reason,
            }
        } else {
            LinkPreviewState::Blocked {
                reason: self.reason,
            }
        }
    }
}

fn validate_destination_parent(destination: &Path) -> Option<PreviewProblem> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    validate_directory_creatable(parent).map(|problem| PreviewProblem {
        reason: format!("destination parent: {}", problem.reason),
        elevation_may_fix: problem.elevation_may_fix,
    })
}

fn validate_replacement_paths(
    backup_directory: &Path,
    destination: &Path,
    destination_metadata: &fs::Metadata,
) -> Option<PreviewProblem> {
    if let Some(problem) = validate_directory_creatable(backup_directory) {
        return Some(PreviewProblem {
            reason: format!("backup directory: {}", problem.reason),
            elevation_may_fix: problem.elevation_may_fix,
        });
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if let Some(problem) = validate_existing_directory_writable(parent) {
        return Some(PreviewProblem {
            reason: format!("destination parent: {}", problem.reason),
            elevation_may_fix: problem.elevation_may_fix,
        });
    }
    if let Some(problem) = validate_cross_device_file_backup_readable(
        backup_directory,
        destination,
        destination_metadata,
    ) {
        return Some(problem);
    }
    validate_sticky_directory_replacement(parent, destination, destination_metadata).map(
        |problem| PreviewProblem {
            reason: format!("destination parent: {}", problem.reason),
            elevation_may_fix: problem.elevation_may_fix,
        },
    )
}

fn validate_directory_creatable(path: &Path) -> Option<PreviewProblem> {
    let mut candidate = path;

    loop {
        match fs::metadata(candidate) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Some(PreviewProblem::blocked(format!(
                        "{} exists and is not a directory",
                        candidate.display()
                    )));
                }
                return validate_existing_directory_writable(candidate);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                match fs::symlink_metadata(candidate) {
                    Ok(_) => {
                        return Some(PreviewProblem::blocked(format!(
                            "{} exists and is not a directory",
                            candidate.display()
                        )));
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => {}
                    Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                        return Some(PreviewProblem::permission_denied(format!(
                            "failed inspecting {}: {err}",
                            candidate.display()
                        )));
                    }
                    Err(err) => {
                        return Some(PreviewProblem::blocked(format!(
                            "failed inspecting {}: {err}",
                            candidate.display()
                        )));
                    }
                }
                let Some(parent) = candidate.parent() else {
                    return Some(PreviewProblem::blocked(format!(
                        "failed finding existing parent for {}",
                        path.display()
                    )));
                };
                if parent == candidate {
                    return Some(PreviewProblem::blocked(format!(
                        "failed finding existing parent for {}",
                        path.display()
                    )));
                }
                candidate = parent;
            }
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                return Some(PreviewProblem::permission_denied(format!(
                    "failed inspecting {}: {err}",
                    candidate.display()
                )));
            }
            Err(err) => {
                return Some(PreviewProblem::blocked(format!(
                    "failed inspecting {}: {err}",
                    candidate.display()
                )));
            }
        }
    }
}

#[cfg(unix)]
fn validate_sticky_directory_replacement(
    parent: &Path,
    destination: &Path,
    destination_metadata: &fs::Metadata,
) -> Option<PreviewProblem> {
    use std::os::unix::fs::MetadataExt;

    let parent_metadata = match fs::metadata(parent) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            return Some(PreviewProblem::permission_denied(format!(
                "failed inspecting {}: {err}",
                parent.display()
            )));
        }
        Err(err) => {
            return Some(PreviewProblem::blocked(format!(
                "failed inspecting {}: {err}",
                parent.display()
            )));
        }
    };
    let euid = unsafe { libc::geteuid() };
    if sticky_directory_allows_replacement(
        parent_metadata.mode(),
        parent_metadata.uid(),
        destination_metadata.uid(),
        euid,
    ) {
        return None;
    }

    Some(PreviewProblem::permission_denied(format!(
        "{} is sticky and the current user does not own {}",
        parent.display(),
        destination.display()
    )))
}

#[cfg(unix)]
fn sticky_directory_allows_replacement(
    parent_mode: u32,
    parent_uid: u32,
    destination_uid: u32,
    euid: u32,
) -> bool {
    parent_mode & sticky_bit() == 0 || euid == 0 || destination_uid == euid || parent_uid == euid
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn sticky_bit() -> u32 {
    libc::S_ISVTX
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn sticky_bit() -> u32 {
    libc::S_ISVTX as u32
}

#[cfg(not(unix))]
fn validate_sticky_directory_replacement(
    _parent: &Path,
    _destination: &Path,
    _destination_metadata: &fs::Metadata,
) -> Option<PreviewProblem> {
    None
}

#[cfg(unix)]
fn validate_cross_device_file_backup_readable(
    backup_directory: &Path,
    destination: &Path,
    destination_metadata: &fs::Metadata,
) -> Option<PreviewProblem> {
    use std::os::unix::fs::MetadataExt;

    if !destination_metadata.is_file() {
        return None;
    }

    let destination_parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let destination_parent_metadata = match fs::metadata(destination_parent) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            return Some(PreviewProblem::permission_denied(format!(
                "destination parent: failed inspecting {}: {err}",
                destination_parent.display()
            )));
        }
        Err(err) => {
            return Some(PreviewProblem::blocked(format!(
                "destination parent: failed inspecting {}: {err}",
                destination_parent.display()
            )));
        }
    };
    let backup_device = match creatable_directory_device(backup_directory) {
        Ok(device) => device,
        Err(problem) => {
            return Some(PreviewProblem {
                reason: format!("backup directory: {}", problem.reason),
                elevation_may_fix: problem.elevation_may_fix,
            });
        }
    };

    if destination_parent_metadata.dev() != backup_device && !file_is_readable(destination) {
        return Some(PreviewProblem::permission_denied(format!(
            "{} is not readable; cross-device backups may need to copy it",
            destination.display()
        )));
    }

    None
}

#[cfg(not(unix))]
fn validate_cross_device_file_backup_readable(
    _backup_directory: &Path,
    _destination: &Path,
    _destination_metadata: &fs::Metadata,
) -> Option<PreviewProblem> {
    None
}

#[cfg(unix)]
fn creatable_directory_device(path: &Path) -> std::result::Result<u64, PreviewProblem> {
    use std::os::unix::fs::MetadataExt;

    let mut candidate = path;

    loop {
        match fs::metadata(candidate) {
            Ok(metadata) => return Ok(metadata.dev()),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                match fs::symlink_metadata(candidate) {
                    Ok(_) => {
                        return Err(PreviewProblem::blocked(format!(
                            "{} exists and is not a directory",
                            candidate.display()
                        )));
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => {}
                    Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                        return Err(PreviewProblem::permission_denied(format!(
                            "failed inspecting {}: {err}",
                            candidate.display()
                        )));
                    }
                    Err(err) => {
                        return Err(PreviewProblem::blocked(format!(
                            "failed inspecting {}: {err}",
                            candidate.display()
                        )));
                    }
                }
                let Some(parent) = candidate.parent() else {
                    return Err(PreviewProblem::blocked(format!(
                        "failed finding existing parent for {}",
                        path.display()
                    )));
                };
                if parent == candidate {
                    return Err(PreviewProblem::blocked(format!(
                        "failed finding existing parent for {}",
                        path.display()
                    )));
                }
                candidate = parent;
            }
            Err(err) if err.kind() == ErrorKind::PermissionDenied => {
                return Err(PreviewProblem::permission_denied(format!(
                    "failed inspecting {}: {err}",
                    candidate.display()
                )));
            }
            Err(err) => {
                return Err(PreviewProblem::blocked(format!(
                    "failed inspecting {}: {err}",
                    candidate.display()
                )));
            }
        }
    }
}

fn validate_existing_directory_writable(path: &Path) -> Option<PreviewProblem> {
    if directory_is_writable(path) {
        None
    } else {
        Some(PreviewProblem::permission_denied(format!(
            "{} is not writable",
            path.display()
        )))
    }
}

#[cfg(unix)]
fn directory_is_writable(path: &Path) -> bool {
    path_has_access(path, libc::W_OK | libc::X_OK)
}

#[cfg(unix)]
fn file_is_readable(path: &Path) -> bool {
    path_has_access(path, libc::R_OK)
}

#[cfg(unix)]
fn path_has_access(path: &Path, mode: libc::c_int) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}

#[cfg(not(unix))]
fn directory_is_writable(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => !metadata.permissions().readonly(),
        Err(_) => false,
    }
}

fn should_defer_to_elevation(
    scope: LinkScope,
    options: PreviewOptions,
    error_kind: ErrorKind,
) -> bool {
    should_defer_problem_to_elevation(scope, options, error_kind == ErrorKind::PermissionDenied)
}

fn should_defer_problem_to_elevation(
    scope: LinkScope,
    options: PreviewOptions,
    elevation_may_fix: bool,
) -> bool {
    scope == LinkScope::System && options.system_links_may_use_elevation && elevation_may_fix
}

fn unique_backup_path(backup_directory: &Path, destination: &Path) -> Result<PathBuf> {
    fs::create_dir_all(backup_directory)
        .with_context(|| format!("failed creating {}", backup_directory.display()))?;

    let timestamp = backup_timestamp(SystemTime::now())?;
    let name = backup_name(destination);
    let hash = path_hash(destination);
    let backup = backup_directory.join(format!("{name}.{timestamp}.{hash:016x}.bak"));

    match fs::symlink_metadata(&backup) {
        Ok(_) => bail!("backup path {} already exists", backup.display()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(backup),
        Err(err) => Err(err).with_context(|| format!("failed inspecting {}", backup.display())),
    }
}

fn backup_timestamp(now: SystemTime) -> Result<String> {
    let timestamp = OffsetDateTime::from(now)
        .format(&Rfc3339)
        .context("failed formatting backup timestamp")?;
    Ok(timestamp.replace(':', "-"))
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

    #[test]
    fn elevated_system_preview_defers_permission_problem() {
        let state = PreviewProblem::permission_denied("root-only").into_preview_state(
            LinkScope::System,
            PreviewOptions {
                system_links_may_use_elevation: true,
            },
        );

        assert!(matches!(state, LinkPreviewState::NeedsElevation { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn sticky_directory_replacement_requires_entry_or_directory_owner() {
        let sticky_mode = sticky_bit() | 0o777;

        assert!(!sticky_directory_allows_replacement(
            sticky_mode,
            1000,
            1001,
            1002
        ));
        assert!(sticky_directory_allows_replacement(
            sticky_mode,
            1000,
            1001,
            1001
        ));
        assert!(sticky_directory_allows_replacement(
            sticky_mode,
            1000,
            1001,
            1000
        ));
        assert!(sticky_directory_allows_replacement(
            sticky_mode,
            1000,
            1001,
            0
        ));
        assert!(sticky_directory_allows_replacement(0o777, 1000, 1001, 1002));
    }

    #[test]
    fn host_matching_accepts_short_and_fully_qualified_forms() {
        assert!(host_matches(
            &[String::from("workstation")],
            "workstation.example.test"
        ));
        assert!(host_matches(
            &[String::from("workstation.example.test")],
            "workstation.example.test"
        ));
        assert!(host_matches(
            &[String::from("WORKSTATION")],
            "workstation.example.test"
        ));
        assert!(host_matches(
            &[String::from("workstation.example.test")],
            "workstation"
        ));
        assert!(host_matches(&[String::from("workstation")], "workstation"));
        assert!(!host_matches(
            &[String::from("workstation.other.test")],
            "workstation.example.test"
        ));
    }

    #[test]
    fn preview_respects_host_filters() {
        let mut symlinks = BTreeMap::new();
        symlinks.insert(
            PathBuf::from("/definitely/missing/source"),
            LinkConfig {
                destinations: vec![PathBuf::from("/definitely/missing/destination")],
                hosts: vec![String::from("definitely-not-this-host.invalid")],
            },
        );
        let config = DotConf {
            backup_directory: PathBuf::from("/tmp/dot-conf-backups"),
            symlinks,
            sys_symlinks: BTreeMap::new(),
        };

        let previews = config.preview(Scope::User).unwrap();

        assert!(previews.is_empty());
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
