#![deny(missing_docs)]
//! Apply dotfile symlink configurations described by YAML.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
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
    /// The source path is missing, so the link cannot be applied.
    MissingSource,
    /// The destination is absent, so a new symlink would be created.
    Create,
    /// The destination already links to the configured source.
    Unchanged,
    /// The destination is a file and would be backed up before replacement.
    ReplaceFile {
        /// The directory where the existing file would be backed up.
        backup_directory: PathBuf,
    },
    /// The destination is a directory and would be backed up before replacement.
    ReplaceDirectory {
        /// The directory where the existing directory would be backed up.
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
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawLink {
    Destinations(OneOrMany<PathBuf>),
    Options(RawLinkOptions),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLinkOptions {
    #[serde(alias = "destination")]
    destinations: OneOrMany<PathBuf>,
    #[serde(default, alias = "host")]
    hosts: OneOrMany<String>,
}

impl RawLink {
    fn into_parts(self) -> (OneOrMany<PathBuf>, Vec<String>) {
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

        let source_base = config_path
            .parent()
            .context("config file has no parent directory")?;
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
        let raw: RawConfig = serde_saphyr::from_str(yaml).context("failed parsing YAML")?;
        let backup_directory = resolve_against(source_base, &raw.backup_directory)
            .with_context(|| format!("failed resolving {}", raw.backup_directory.display()))?;
        let symlinks = normalize_links(source_base, destination_base, raw.symlinks)?;
        let sys_symlinks = normalize_links(source_base, destination_base, raw.sys_symlinks)?;
        validate_unique_destinations(&symlinks, &sys_symlinks)?;

        Ok(Self {
            backup_directory,
            symlinks,
            sys_symlinks,
        })
    }

    fn sections(
        &self,
        scope: Scope,
    ) -> impl Iterator<Item = (LinkScope, &BTreeMap<PathBuf, LinkConfig>)> {
        [
            (LinkScope::System, &self.sys_symlinks),
            (LinkScope::User, &self.symlinks),
        ]
        .into_iter()
        .filter(move |(section, _)| {
            matches!(
                (scope, section),
                (Scope::All, _) | (Scope::Sys, LinkScope::System) | (Scope::User, LinkScope::User)
            )
        })
    }

    fn active_links(&self, scope: Scope) -> Result<Vec<(LinkScope, &Path, &Path)>> {
        let mut current_host = None;
        let mut links = Vec::new();
        for (section, sources) in self.sections(scope) {
            for (source, link) in sources {
                if link_applies_to_current_host(link, &mut current_host)? {
                    for destination in &link.destinations {
                        links.push((section, source.as_path(), destination.as_path()));
                    }
                }
            }
        }
        Ok(links)
    }

    /// Validate destinations and protected source/backup paths across configurations.
    ///
    /// Selected, host-matching links may not overlap or refer to the same destination
    /// through different path spellings. Replacing a destination must not move a
    /// configured source or backup directory out of the way.
    pub fn validate_configs(configs: &[&Self], scope: Scope) -> Result<()> {
        let mut destinations = BTreeMap::new();
        let mut protected = Vec::new();
        for config in configs {
            let links = config.active_links(scope)?;
            if links.is_empty() {
                continue;
            }
            protected.push((
                "backup directory",
                config.backup_directory.as_path(),
                protected_paths(&config.backup_directory)?,
            ));
            for (_, source, destination) in links {
                let destination_id = identity_key(path_identity(destination)?);
                if destination_id.parent().is_none() {
                    bail!("cannot replace filesystem root {}", destination.display());
                }
                if let Some(previous) = destinations.insert(destination_id, destination) {
                    bail!(
                        "destination {} is configured more than once (also {})",
                        destination.display(),
                        previous.display()
                    );
                }
                protected.push(("source", source, protected_paths(source)?));
                let parent = destination.parent().context("destination has no parent")?;
                protected.push(("destination parent", parent, protected_paths(parent)?));
            }
        }
        for (destination, original) in &destinations {
            for parent in destination.ancestors().skip(1) {
                if destinations.contains_key(parent) {
                    bail!(
                        "overlapping destinations: {} and {}",
                        parent.display(),
                        original.display()
                    );
                }
            }
        }
        for (destination, original) in &destinations {
            for (kind, path, dependencies) in &protected {
                if dependencies
                    .iter()
                    .any(|dependency| dependency.starts_with(destination))
                {
                    if *kind == "source" && identity_key(path_identity(path)?) == *destination {
                        bail!(
                            "source and destination are the same path: {}",
                            original.display()
                        );
                    }
                    bail!(
                        "destination {} would move or retarget {} {}",
                        original.display(),
                        kind,
                        path.display()
                    );
                }
            }
        }
        Ok(())
    }

    /// Inspect selected links without changing files.
    ///
    /// Structural conflicts fail the whole plan. Filesystem blockers are returned
    /// per link so a dry run can report all of them.
    pub fn preview(&self, scope: Scope) -> Result<Vec<LinkPreview>> {
        Self::validate_configs(&[self], scope)?;
        self.active_links(scope)?
            .into_iter()
            .map(|(scope, source, destination)| {
                let state = match inspect_destination(&self.backup_directory, source, destination) {
                    Ok(state) => state,
                    Err(err) => LinkPreviewState::Blocked {
                        reason: format!("{err:#}"),
                    },
                };
                Ok(LinkPreview {
                    scope,
                    source: source.to_path_buf(),
                    destination: destination.to_path_buf(),
                    state,
                })
            })
            .collect()
    }

    /// Apply selected links after checking the entire scope for blockers.
    ///
    /// Existing destinations are backed up before replacement. Runtime failures
    /// can leave earlier links applied; application is not a transaction.
    pub fn apply(&self, scope: Scope) -> Result<()> {
        let previews = self.preview(scope)?;
        for preview in &previews {
            preview.ensure_applicable()?;
        }
        for preview in previews {
            // Inspect again immediately before each mutation; a preview is not a lock.
            let state = inspect_destination(
                &self.backup_directory,
                &preview.source,
                &preview.destination,
            )?;
            if state == LinkPreviewState::MissingSource {
                bail!("source missing: {}", preview.source.display());
            }
            if state == LinkPreviewState::Unchanged {
                continue;
            }
            let parent = preview
                .destination
                .parent()
                .context("destination has no parent")?;
            fs::create_dir_all(parent)
                .with_context(|| format!("failed creating {}", parent.display()))?;
            backup_and_remove_if_exists(&self.backup_directory, &preview.destination)?;
            create_symlink(&preview.source, &preview.destination)?;
            log::info!(
                "{} -> {}",
                preview.destination.display(),
                preview.source.display()
            );
        }
        Ok(())
    }
}

impl LinkPreview {
    /// Fail when this link cannot be applied under the current credentials.
    pub fn ensure_applicable(&self) -> Result<()> {
        match &self.state {
            LinkPreviewState::MissingSource => bail!("source missing: {}", self.source.display()),
            LinkPreviewState::Blocked { reason } => {
                bail!("{}: {}", self.destination.display(), reason)
            }
            _ => Ok(()),
        }
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
        if resolved_destinations.is_empty() {
            bail!("source {} has no destinations", source.display());
        }
        if hosts.iter().any(|host| host.trim().is_empty()) {
            bail!("source {} has an empty hostname", source.display());
        }
        if normalized
            .insert(
                resolved_source.clone(),
                LinkConfig {
                    destinations: resolved_destinations,
                    hosts,
                },
            )
            .is_some()
        {
            bail!(
                "source {} is configured more than once after path resolution",
                resolved_source.display()
            );
        }
    }
    Ok(normalized)
}

fn validate_unique_destinations(
    symlinks: &BTreeMap<PathBuf, LinkConfig>,
    sys_symlinks: &BTreeMap<PathBuf, LinkConfig>,
) -> Result<()> {
    let mut seen = BTreeMap::new();
    for (scope, links) in [
        (LinkScope::User, symlinks),
        (LinkScope::System, sys_symlinks),
    ] {
        for (source, link) in links {
            for destination in &link.destinations {
                if let Some((previous_scope, previous_source)) =
                    seen.insert(destination.clone(), (scope, source.clone()))
                {
                    bail!(
                        "destination {} is configured more than once ({} source {} and {} source {})",
                        destination.display(),
                        section_name(previous_scope),
                        previous_source.display(),
                        section_name(scope),
                        source.display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn section_name(scope: LinkScope) -> &'static str {
    match scope {
        LinkScope::User => "symlinks",
        LinkScope::System => "sys_symlinks",
    }
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
    let (current_short, _) = hostname_parts(current_host);

    hosts.iter().any(|host| {
        let (_, host_is_fqdn) = hostname_parts(host);
        if host_is_fqdn {
            host.eq_ignore_ascii_case(current_host)
        } else {
            host.eq_ignore_ascii_case(current_short)
        }
    })
}

fn hostname_parts(hostname: &str) -> (&str, bool) {
    hostname
        .split_once('.')
        .map_or((hostname, false), |(short, _)| (short, true))
}

fn link_applies_to_current_host(
    link: &LinkConfig,
    current_host: &mut Option<String>,
) -> Result<bool> {
    if link.hosts.is_empty() {
        return Ok(true);
    }
    if current_host.is_none() {
        *current_host = Some(current_hostname()?);
    }
    let hostname = current_host.as_deref().expect("hostname was initialized");
    Ok(host_matches(&link.hosts, hostname))
}

fn resolve_from_cwd(path: &Path) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed reading current directory")?;
    resolve_against(&cwd, path)
}

fn resolve_against(base: &Path, path: &Path) -> Result<PathBuf> {
    let expanded = expand_tilde(path)?;
    #[cfg(windows)]
    if let Some(name) = expanded.file_name() {
        let name = name.to_string_lossy();
        let stem = name.split('.').next().context("filename has no stem")?;
        let short_name = stem.rsplit_once('~').is_some_and(|(_, suffix)| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        });
        if name.ends_with(['.', ' ']) || short_name {
            bail!(
                "Windows paths must use long filenames without trailing dots or spaces: {}",
                expanded.display()
            );
        }
    }
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    })
}

fn expand_tilde(path: &Path) -> Result<PathBuf> {
    expand_tilde_with_home(path, home::home_dir().as_deref())
}

fn expand_tilde_with_home(path: &Path, home: Option<&Path>) -> Result<PathBuf> {
    let mut components = path.components();
    let Some(first) = components.next() else {
        bail!("path must not be empty");
    };
    if first.as_os_str() == "~" {
        let home = home.context("cannot expand ~: current user's home directory is unavailable")?;
        return Ok(home.join(components.as_path()));
    }
    if first.as_os_str().to_string_lossy().starts_with('~') {
        bail!(
            "unsupported home path {}; use ~ or ~/ for the current user",
            path.display()
        );
    }
    Ok(path.to_path_buf())
}

// Resolve parent aliases without following the final symlink, which is the entry
// that will be replaced. Missing parents are resolved from their nearest ancestor.
fn path_identity(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    match fs::symlink_metadata(&absolute) {
        Ok(metadata) if !metadata.file_type().is_symlink() => {
            return absolute
                .canonicalize()
                .with_context(|| format!("failed resolving {}", absolute.display()));
        }
        Ok(_) => {}
        Err(err) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {}
        Err(err) => {
            return Err(err).with_context(|| format!("failed inspecting {}", absolute.display()))
        }
    }
    if absolute.file_name().is_none() {
        return absolute
            .canonicalize()
            .with_context(|| format!("failed resolving {}", absolute.display()));
    }
    let mut ancestor = absolute.parent().context("path has no parent")?;
    let mut suffix = Vec::new();
    let mut resolved = loop {
        match ancestor.canonicalize() {
            Ok(parent) => break parent,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                match fs::symlink_metadata(ancestor) {
                    Ok(_) => bail!("dangling parent symlink {}", ancestor.display()),
                    Err(err) if err.kind() == ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(err)
                            .with_context(|| format!("failed inspecting {}", ancestor.display()))
                    }
                }
                suffix.push(
                    ancestor
                        .file_name()
                        .with_context(|| format!("invalid parent {}", ancestor.display()))?,
                );
                ancestor = ancestor.parent().context("path has no existing ancestor")?;
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed resolving parent {}", ancestor.display()))
            }
        }
    };
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    if let Some(name) = absolute.file_name() {
        resolved.push(name);
    }
    Ok(resolved)
}

// macOS and Windows configs reject case/Unicode aliases conservatively, including
// absent destinations, so the first created link cannot hide the second mapping.
fn identity_key(path: PathBuf) -> PathBuf {
    #[cfg(any(target_os = "macos", windows))]
    {
        use caseless::Caseless;
        use unicode_normalization::UnicodeNormalization;
        PathBuf::from(
            path.to_string_lossy()
                .nfd()
                .default_case_fold()
                .nfd()
                .collect::<String>(),
        )
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        path
    }
}

fn protected_paths(path: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut pending = vec![std::path::absolute(path)?];
    let mut visited = BTreeSet::new();
    let mut protected = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !visited.insert(path_identity(&path)?) {
            continue;
        }
        for entry in path.ancestors() {
            protected.insert(identity_key(path_identity(entry)?));
            match fs::symlink_metadata(entry) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    pending.push(symlink_backup_target(entry)?);
                }
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed inspecting {}", entry.display()))
                }
            }
        }
    }
    Ok(protected)
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

    if metadata.is_dir() {
        fs::rename(destination, &backup)
            .with_context(|| format!("failed moving {}", destination.display()))?;
        return Ok(());
    }

    bail!(
        "destination {} exists and is not a file/directory/symlink",
        destination.display()
    )
}

fn inspect_destination(
    backup_directory: &Path,
    source: &Path,
    destination: &Path,
) -> Result<LinkPreviewState> {
    let source_metadata = match fs::metadata(source) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(LinkPreviewState::MissingSource)
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed inspecting source {}", source.display()))
        }
    };
    if source_metadata.is_dir() {
        let source_id = identity_key(source.canonicalize()?);
        if identity_key(path_identity(destination)?).starts_with(&source_id) {
            bail!(
                "destination {} is inside source directory {}",
                destination.display(),
                source.display()
            );
        }
    }
    let metadata = match fs::symlink_metadata(destination) {
        Ok(metadata) => metadata,
        Err(err) if matches!(err.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            validate_directory_creatable(
                destination.parent().context("destination has no parent")?,
            )
            .context("destination parent")?;
            return Ok(LinkPreviewState::Create);
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed inspecting destination {}", destination.display())
            })
        }
    };
    let target = if metadata.file_type().is_symlink() {
        let target = symlink_backup_target(destination)?;
        if path_resolves_to_same_file(&target, source)? {
            return Ok(LinkPreviewState::Unchanged);
        }
        Some(target)
    } else {
        None
    };
    if !(metadata.is_file() || metadata.is_dir() || target.is_some()) {
        bail!("destination exists and is not a file/directory/symlink");
    }
    let backup_metadata =
        validate_directory_creatable(backup_directory).context("backup directory")?;
    let parent = destination.parent().context("destination has no parent")?;
    let parent_metadata = validate_directory_creatable(parent).context("destination parent")?;
    validate_backup(
        backup_directory,
        destination,
        &metadata,
        &backup_metadata,
        &parent_metadata,
    )?;
    let backup_directory = backup_directory.to_path_buf();
    Ok(if let Some(target) = target {
        LinkPreviewState::ReplaceSymlink {
            backup_directory,
            target,
        }
    } else if metadata.is_dir() {
        LinkPreviewState::ReplaceDirectory { backup_directory }
    } else {
        LinkPreviewState::ReplaceFile { backup_directory }
    })
}

fn validate_directory_creatable(path: &Path) -> Result<fs::Metadata> {
    for candidate in path.ancestors() {
        match fs::metadata(candidate) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    bail!("{} exists and is not a directory", candidate.display());
                }
                directory_is_writable(candidate, &metadata)?;
                return Ok(metadata);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                match fs::symlink_metadata(candidate) {
                    Ok(_) => bail!("{} exists and is not a directory", candidate.display()),
                    Err(err) if err.kind() == ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(err)
                            .with_context(|| format!("failed inspecting {}", candidate.display()))
                    }
                }
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed inspecting {}", candidate.display()))
            }
        }
    }
    bail!("no existing parent for {}", path.display())
}

#[cfg(unix)]
fn validate_backup(
    backup_directory: &Path,
    destination: &Path,
    metadata: &fs::Metadata,
    backup_metadata: &fs::Metadata,
    parent_metadata: &fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let euid = unsafe { libc::geteuid() };
    if !sticky_directory_allows_replacement(
        parent_metadata.mode(),
        parent_metadata.uid(),
        metadata.uid(),
        euid,
    ) {
        bail!(
            "destination parent is sticky and the current user does not own {}",
            destination.display()
        );
    }
    if metadata.dev() != backup_metadata.dev() && !metadata.file_type().is_symlink() {
        if metadata.is_dir() {
            bail!(
                "cannot move directory {} across filesystems to {}",
                destination.display(),
                backup_directory.display()
            );
        }
        path_has_access(destination, libc::R_OK)
            .context("cross-device file backup requires read access")?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_backup(
    _: &Path,
    _: &Path,
    _: &fs::Metadata,
    _: &fs::Metadata,
    _: &fs::Metadata,
) -> Result<()> {
    Ok(())
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

#[cfg(unix)]
fn directory_is_writable(path: &Path, _: &fs::Metadata) -> Result<()> {
    path_has_access(path, libc::W_OK | libc::X_OK)
        .with_context(|| format!("{} is not writable or searchable", path.display()))
}

#[cfg(not(unix))]
fn directory_is_writable(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.permissions().readonly() {
        bail!("{} is not writable", path.display());
    }
    Ok(())
}

#[cfg(unix)]
fn path_has_access(path: &Path, mode: libc::c_int) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).context("path contains a null byte")?;
    if unsafe { libc::access(path.as_ptr(), mode) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
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

fn path_resolves_to_same_file(left: &Path, right: &Path) -> Result<bool> {
    match left.canonicalize() {
        Ok(left) => Ok(left
            == right
                .canonicalize()
                .with_context(|| format!("failed resolving {}", right.display()))?),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => {
            Err(err).with_context(|| format!("failed resolving symlink target {}", left.display()))
        }
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
        assert!(!host_matches(
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
