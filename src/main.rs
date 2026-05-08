use clap::Parser;
use dot_conf::{DotConf, Scope};
use std::env;
#[cfg(unix)]
use std::process::Command;

#[derive(Parser, Debug)]
#[command(name = "dot-conf", about = "Apply dot-conf configuration files")]
struct Cli {
    #[arg(required = true)]
    filenames: Vec<String>,
    #[arg(long)]
    sys_only: bool,
    #[arg(long)]
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
            system_config_filenames.push(filename.clone());
        }
    }

    if !system_config_filenames.is_empty() {
        apply_system_config_with_elevation(&system_config_filenames)?;
    }

    if env::var_os("DOTCONF_SUBPROCESS").is_none() {
        println!("Done!");
    }

    Ok(())
}

#[cfg(unix)]
fn is_elevated() -> bool {
    let Ok(output) = Command::new("id").arg("-u").output() else {
        return false;
    };

    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "0"
}

#[cfg(not(unix))]
fn is_elevated() -> bool {
    true
}

#[cfg(unix)]
fn apply_system_config_with_elevation(filenames: &[String]) -> anyhow::Result<()> {
    println!("Enter password here to apply system config:");
    let executable = env::current_exe()?;
    let status = Command::new("sudo")
        .arg("-E")
        .arg(executable)
        .arg("--sys-only")
        .args(filenames)
        .env("DOTCONF_SUBPROCESS", "true")
        .status()?;

    let _ = Command::new("sudo").arg("-k").status();

    if !status.success() {
        anyhow::bail!("system config apply failed with status {status}");
    }

    Ok(())
}

#[cfg(not(unix))]
fn apply_system_config_with_elevation(_filenames: &[String]) -> anyhow::Result<()> {
    anyhow::bail!(
        "system config requires elevated privileges; rerun with --sys-only from an elevated shell"
    )
}
