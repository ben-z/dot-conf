use anyhow::Context;
use clap::{ArgAction, Parser, ValueEnum};
use dot_conf::{DotConf, LinkPreview, LinkPreviewState, LinkScope, Scope};
use log::LevelFilter;
#[cfg(unix)]
use std::env;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

#[derive(Parser, Debug)]
#[command(
    name = "dot-conf",
    version,
    about = "Apply dot-conf configuration files",
    long_about = "Apply dot-conf YAML configs by creating symlinks and backing up existing file or symlink destinations before replacement."
)]
struct Cli {
    #[arg(
        value_name = "CONFIG",
        required = true,
        help = "YAML config file(s) to apply"
    )]
    filenames: Vec<PathBuf>,

    #[arg(
        long,
        value_enum,
        default_value = "all",
        help = "Which config section(s) to apply"
    )]
    scope: CliScope,

    #[arg(
        long,
        alias = "system-only",
        conflicts_with_all = ["scope", "user_only"],
        help = "Apply only sys_symlinks (alias for --scope system)"
    )]
    sys_only: bool,

    #[arg(
        long,
        conflicts_with_all = ["scope", "sys_only"],
        help = "Apply only symlinks (alias for --scope user)"
    )]
    user_only: bool,

    #[arg(
        long,
        help = "Preview resolved changes without modifying files or invoking sudo"
    )]
    dry_run: bool,

    #[arg(
        short,
        long,
        action = ArgAction::Count,
        conflicts_with = "quiet",
        help = "Increase log verbosity (-v for info, -vv for debug)"
    )]
    verbose: u8,

    #[arg(
        short,
        long,
        action = ArgAction::Count,
        conflicts_with = "verbose",
        help = "Reduce log output (-q shows only errors, -qq disables logs)"
    )]
    quiet: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliScope {
    All,
    User,
    System,
}

impl From<CliScope> for Scope {
    fn from(scope: CliScope) -> Self {
        match scope {
            CliScope::All => Self::All,
            CliScope::User => Self::User,
            CliScope::System => Self::Sys,
        }
    }
}

impl Cli {
    fn resolved_scope(&self) -> Scope {
        if self.sys_only {
            Scope::Sys
        } else if self.user_only {
            Scope::User
        } else {
            self.scope.into()
        }
    }

    fn log_level_filter(&self) -> LevelFilter {
        match self.quiet {
            0 => match self.verbose {
                0 => LevelFilter::Warn,
                1 => LevelFilter::Info,
                2 => LevelFilter::Debug,
                _ => LevelFilter::Trace,
            },
            1 => LevelFilter::Error,
            _ => LevelFilter::Off,
        }
    }
}

struct NamedConfig {
    filename: PathBuf,
    config: DotConf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_logger(&cli);

    let configs = load_configs(&cli.filenames)?;
    let scope = cli.resolved_scope();

    if cli.dry_run {
        return print_dry_run(&configs, scope);
    }

    apply_configs(&configs, scope)
}

fn init_logger(cli: &Cli) {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"));
    if cli.verbose > 0 || cli.quiet > 0 {
        builder.filter_level(cli.log_level_filter());
    }
    builder.format_timestamp(None).init();
}

fn load_configs(filenames: &[PathBuf]) -> anyhow::Result<Vec<NamedConfig>> {
    filenames
        .iter()
        .map(|filename| {
            Ok(NamedConfig {
                filename: filename.clone(),
                config: DotConf::from_yaml_file(filename)?,
            })
        })
        .collect()
}

fn apply_configs(configs: &[NamedConfig], scope: Scope) -> anyhow::Result<()> {
    if scope == Scope::All
        && configs.iter().any(|config| config.config.requires_root())
        && !is_elevated()
    {
        let system_config_filenames: Vec<_> = configs
            .iter()
            .filter(|config| config.config.requires_root())
            .map(|config| config.filename.clone())
            .collect();
        apply_system_config_with_elevation(&system_config_filenames)?;
        return apply_scope(configs, Scope::User);
    }

    apply_scope(configs, scope)
}

fn apply_scope(configs: &[NamedConfig], scope: Scope) -> anyhow::Result<()> {
    for named in configs {
        named
            .config
            .apply(scope)
            .with_context(|| format!("applying {}", named.filename.display()))?;
    }
    Ok(())
}

fn print_dry_run(configs: &[NamedConfig], scope: Scope) -> anyhow::Result<()> {
    println!("Dry run: no files will be changed.");
    if scope == Scope::All && configs.iter().any(|config| config.config.requires_root()) {
        if is_elevated() {
            println!("System links would be applied before user links.");
        } else {
            println!(
                "System links would be applied first with sudo; user links would be applied only after that succeeds."
            );
        }
    }

    let mut blocked = false;
    for named in configs {
        let previews = named
            .config
            .preview(scope)
            .with_context(|| format!("previewing {}", named.filename.display()))?;
        println!();
        println!("{}:", named.filename.display());
        if previews.is_empty() {
            println!("  no links for {}", scope_label(scope));
            continue;
        }
        for preview in previews {
            blocked |= matches!(&preview.state, LinkPreviewState::Blocked { .. });
            print_preview(&preview);
        }
    }

    if blocked {
        anyhow::bail!("dry-run found destinations that cannot be replaced");
    }

    Ok(())
}

fn print_preview(preview: &LinkPreview) {
    let scope = match preview.scope {
        LinkScope::User => "user",
        LinkScope::System => "system",
    };
    let source = preview.source.display();
    let destination = preview.destination.display();

    match &preview.state {
        LinkPreviewState::MissingSource => {
            println!("  [{scope}] skip {destination} -> {source} (source missing)");
        }
        LinkPreviewState::Create => {
            println!("  [{scope}] create {destination} -> {source}");
        }
        LinkPreviewState::ReplaceFile { backup_directory } => {
            println!("  [{scope}] replace file {destination} -> {source}");
            println!("    backup directory: {}", backup_directory.display());
        }
        LinkPreviewState::ReplaceSymlink {
            backup_directory,
            target,
        } => {
            println!("  [{scope}] replace symlink {destination} -> {source}");
            println!("    existing target: {}", target.display());
            println!("    backup directory: {}", backup_directory.display());
        }
        LinkPreviewState::Blocked { reason } => {
            println!("  [{scope}] blocked {destination} -> {source} ({reason})");
        }
    }
}

fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::All => "all scopes",
        Scope::User => "user scope",
        Scope::Sys => "system scope",
    }
}

#[cfg(unix)]
fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_elevated() -> bool {
    false
}

#[cfg(unix)]
fn apply_system_config_with_elevation(filenames: &[PathBuf]) -> anyhow::Result<()> {
    eprintln!("Applying system config with sudo:");
    let executable = env::current_exe()?;
    let status = Command::new("sudo")
        .arg("-E")
        .arg(executable)
        .arg("--scope")
        .arg("system")
        .arg("--")
        .args(filenames.iter().map(|path| path.as_os_str()))
        .status()?;

    let _ = Command::new("sudo").arg("-k").status();

    if !status.success() {
        anyhow::bail!("system config apply failed with status {status}");
    }

    Ok(())
}

#[cfg(not(unix))]
fn apply_system_config_with_elevation(_filenames: &[PathBuf]) -> anyhow::Result<()> {
    anyhow::bail!(
        "system config requires elevated privileges; rerun with --scope system from an elevated shell"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn rejects_sys_only_with_user_only() {
        let result = Cli::try_parse_from(["dot-conf", "--sys-only", "--user-only", "config.yaml"]);
        assert!(result.is_err());
    }

    #[test]
    fn resolves_scope_flag() {
        let cli = Cli::try_parse_from(["dot-conf", "--scope", "system", "config.yaml"]).unwrap();
        assert_eq!(cli.resolved_scope(), Scope::Sys);

        let cli = Cli::try_parse_from(["dot-conf", "--scope", "user", "config.yaml"]).unwrap();
        assert_eq!(cli.resolved_scope(), Scope::User);
    }

    #[test]
    fn rejects_scope_with_legacy_scope_flag() {
        let result = Cli::try_parse_from([
            "dot-conf",
            "--scope",
            "system",
            "--user-only",
            "config.yaml",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn help_includes_actionable_options() {
        let mut command = Cli::command();
        let help = command.render_long_help().to_string();

        assert!(help.contains("--scope <SCOPE>"));
        assert!(help.contains("--dry-run"));
        assert!(help.contains("--version"));
        assert!(help.contains("<CONFIG>..."));
    }
}
