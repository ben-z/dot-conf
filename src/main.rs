use clap::Parser;
use dot_conf::{DotConf, Scope};

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

    for filename in cli.filenames {
        let config = DotConf::from_yaml_file(filename)?;
        if cli.sys_only {
            config.apply(Scope::Sys)?;
        } else if cli.user_only {
            config.apply(Scope::User)?;
        } else {
            config.apply(Scope::All)?;
        }
    }

    println!("Done!");
    Ok(())
}
