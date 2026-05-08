use clap::Parser;
use dot_conf::{DotConf, Scope};
#[cfg(unix)]
use std::env;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "dot-conf", about = "Apply dot-conf configuration files")]
struct Cli {
    #[arg(required = true)]
    filenames: Vec<PathBuf>,
    #[arg(long, conflicts_with = "user_only")]
    sys_only: bool,
    #[arg(long, conflicts_with = "sys_only")]
    user_only: bool,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let mut system_config_filenames = Vec::new();

    for filename in &cli.filenames {
        let config = DotConf::from_yaml_file(filename)?;
        if cli.sys_only {
            config.apply(Scope::Sys)?;
        } else if cli.user_only {
            config.apply(Scope::User)?;
        } else if !config.requires_root() || is_elevated() {
            config.apply(Scope::All)?;
        } else {
            config.apply(Scope::User)?;
            system_config_filenames.push(filename);
        }
    }

    if !system_config_filenames.is_empty() {
        apply_system_config_with_elevation(&system_config_filenames)?;
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
}
