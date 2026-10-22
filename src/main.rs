use anyhow::Context;
use clap::{ArgAction, Parser, ValueEnum};
use dot_conf::{DotConf, LinkPreview, LinkPreviewState, LinkScope, Scope};
use log::LevelFilter;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "dot-conf",
    version,
    about = "Apply dot-conf configuration files",
    long_about = "Apply dot-conf YAML configs by creating symlinks and backing up existing file, directory, or symlink destinations before replacement."
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

    let scope = cli.resolved_scope();
    let configs = load_configs(&cli.filenames)?;
    DotConf::validate_configs(
        &configs
            .iter()
            .map(|named| &named.config)
            .collect::<Vec<_>>(),
        scope,
    )?;

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
    for named in configs {
        for preview in named.config.preview(scope)? {
            preview
                .ensure_applicable()
                .with_context(|| format!("checking {}", named.filename.display()))?;
        }
    }
    match scope {
        Scope::All => {
            apply_scope(configs, Scope::Sys)?;
            apply_scope(configs, Scope::User)
        }
        _ => apply_scope(configs, scope),
    }
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
    let mut output = io::stdout().lock();
    writeln!(output, "Dry run: no files will be changed.")?;
    let mut blocked = false;
    for named in configs {
        let previews = named
            .config
            .preview(scope)
            .with_context(|| format!("previewing {}", named.filename.display()))?;
        writeln!(output)?;
        writeln!(output, "{}:", named.filename.display())?;
        if previews.is_empty() {
            writeln!(output, "  no links for {}", scope_label(scope))?;
            continue;
        }
        for preview in previews {
            blocked |= preview.ensure_applicable().is_err();
            print_preview(&mut output, &preview)?;
        }
    }

    if blocked {
        anyhow::bail!("dry-run found links that cannot be applied");
    }

    Ok(())
}

fn print_preview(output: &mut impl Write, preview: &LinkPreview) -> io::Result<()> {
    let scope = match preview.scope {
        LinkScope::User => "user",
        LinkScope::System => "system",
    };
    let source = preview.source.display();
    let destination = preview.destination.display();

    match &preview.state {
        LinkPreviewState::MissingSource => {
            writeln!(
                output,
                "  [{scope}] missing {destination} -> {source} (source missing)"
            )?;
        }
        LinkPreviewState::Create => {
            writeln!(output, "  [{scope}] create {destination} -> {source}")?;
        }
        LinkPreviewState::Unchanged => {
            writeln!(output, "  [{scope}] unchanged {destination} -> {source}")?;
        }
        LinkPreviewState::ReplaceFile { backup_directory } => {
            writeln!(output, "  [{scope}] replace file {destination} -> {source}")?;
            writeln!(
                output,
                "    backup directory: {}",
                backup_directory.display()
            )?;
        }
        LinkPreviewState::ReplaceDirectory { backup_directory } => {
            writeln!(
                output,
                "  [{scope}] replace directory {destination} -> {source}"
            )?;
            writeln!(
                output,
                "    backup directory: {}",
                backup_directory.display()
            )?;
        }
        LinkPreviewState::ReplaceSymlink {
            backup_directory,
            target,
        } => {
            writeln!(
                output,
                "  [{scope}] replace symlink {destination} -> {source}"
            )?;
            writeln!(output, "    existing target: {}", target.display())?;
            writeln!(
                output,
                "    backup directory: {}",
                backup_directory.display()
            )?;
        }
        LinkPreviewState::Blocked { reason } => {
            writeln!(
                output,
                "  [{scope}] blocked {destination} -> {source} ({reason})"
            )?;
        }
    }
    Ok(())
}

fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::All => "all scopes",
        Scope::User => "user scope",
        Scope::Sys => "system scope",
    }
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
