use clap::Parser;
use dot_conf::{DotConf, Scope};
#[cfg(unix)]
use std::env;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

#[derive(Parser, Debug)]
#[command(
    name = "dot-conf",
    version,
    about = "Apply dot-conf configuration files"
)]
struct Cli {
    #[arg(required = true)]
    filenames: Vec<PathBuf>,
    #[arg(long, conflicts_with = "user_only")]
    sys_only: bool,
    #[arg(long, conflicts_with = "sys_only")]
    user_only: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    env_logger::init();
    let configs = cli
        .filenames
        .iter()
        .map(|filename| Ok((filename, DotConf::from_yaml_file(filename)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;

    if cli.sys_only {
        for (_, config) in &configs {
            config.apply(Scope::Sys)?;
        }
        return Ok(());
    }

    if cli.user_only {
        for (_, config) in &configs {
            config.apply(Scope::User)?;
        }
        return Ok(());
    }

    if is_elevated() {
        for (_, config) in &configs {
            config.apply(Scope::All)?;
        }
        return Ok(());
    }

    let system_config_filenames = configs
        .iter()
        .filter_map(|(filename, config)| config.requires_root().then_some(*filename))
        .collect::<Vec<_>>();
    if !system_config_filenames.is_empty() {
        apply_system_config_with_elevation(&system_config_filenames)?;
    }

    for (_, config) in &configs {
        config.apply(Scope::User)?;
    }

    Ok(())
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
fn apply_system_config_with_elevation(filenames: &[&PathBuf]) -> anyhow::Result<()> {
    eprintln!("Enter password here to apply system config:");
    let executable = env::current_exe()?;
    let status = Command::new("sudo")
        .arg("-E")
        .arg(executable)
        .arg("--sys-only")
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
fn apply_system_config_with_elevation(_filenames: &[&PathBuf]) -> anyhow::Result<()> {
    anyhow::bail!(
        "system config requires elevated privileges; rerun with --sys-only from an elevated shell"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sys_only_with_user_only() {
        let result = Cli::try_parse_from(["dot-conf", "--sys-only", "--user-only", "config.yaml"]);
        assert!(result.is_err());
    }

    #[test]
    fn supports_version_flag() {
        let result = Cli::try_parse_from(["dot-conf", "--version"]);

        assert!(result.is_err_and(|err| err.kind() == clap::error::ErrorKind::DisplayVersion));
    }
}
