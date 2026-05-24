use serial_test::serial;
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::tempdir;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn dot_conf_with_home(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dot-conf"));
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("HOMEDRIVE")
        .env_remove("HOMEPATH")
        .env_remove("RUST_LOG");
    command
}

#[cfg(unix)]
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "expected success\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected failure\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn backup_entries(home: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(home.join(".config/backup"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    entries
}

#[cfg(unix)]
#[test]
#[serial]
fn workstation_bootstrap_is_previewable_repeatable_and_preserves_replaced_state() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let dotfiles = root.join("dotfiles");
    let system = root.join("system");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&dotfiles).unwrap();
    fs::create_dir_all(&system).unwrap();

    write_file(&dotfiles.join(".vimrc"), "set number\n");
    write_file(&dotfiles.join(".tmux.conf"), "set -g mouse on\n");
    write_file(&dotfiles.join("nvim/init.lua"), "vim.o.number = true\n");
    write_file(&dotfiles.join("sysctl.conf"), "kern.test=1\n");

    write_file(&home.join(".vimrc"), "old vimrc\n");
    write_file(&home.join(".config/nvim/init.lua"), "old nvim\n");
    let old_tmux_target = home.join("old-tmux.conf");
    write_file(&old_tmux_target, "old tmux\n");
    std::os::unix::fs::symlink(&old_tmux_target, home.join(".tmux.conf")).unwrap();

    let config = dotfiles.join("dot-conf.yaml");
    fs::write(
        &config,
        format!(
            r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
  .tmux.conf:
    - ~/.tmux.conf
    - ~/.config/tmux/tmux.conf
  nvim: ~/.config/nvim
sys_symlinks:
  sysctl.conf: {}
"#,
            system.join("sysctl.conf").display()
        ),
    )
    .unwrap();

    let dry_run = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg(&config)
        .output()
        .unwrap();
    assert_success(&dry_run);
    let stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(stdout.contains("[user] replace file"));
    assert!(stdout.contains("[user] replace symlink"));
    assert!(stdout.contains("[user] replace directory"));
    assert!(stdout.contains("[system] create"));
    assert!(!home.join(".config/backup").exists());

    let apply = dot_conf_with_home(&home)
        .arg("--scope")
        .arg("user")
        .arg(&config)
        .output()
        .unwrap();
    assert_success(&apply);

    assert_eq!(
        home.join(".vimrc").canonicalize().unwrap(),
        dotfiles.join(".vimrc").canonicalize().unwrap()
    );
    assert_eq!(
        home.join(".tmux.conf").canonicalize().unwrap(),
        dotfiles.join(".tmux.conf").canonicalize().unwrap()
    );
    assert_eq!(
        home.join(".config/tmux/tmux.conf").canonicalize().unwrap(),
        dotfiles.join(".tmux.conf").canonicalize().unwrap()
    );
    assert_eq!(
        home.join(".config/nvim").canonicalize().unwrap(),
        dotfiles.join("nvim").canonicalize().unwrap()
    );
    assert!(!system.join("sysctl.conf").exists());

    let backups = backup_entries(&home);
    assert_eq!(backups.len(), 3);
    assert!(backups.iter().any(|path| path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with(".vimrc.")));
    assert!(backups.iter().any(|path| path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with(".tmux.conf.")));
    assert!(backups.iter().any(|path| path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("nvim.")));
    assert!(backups
        .iter()
        .any(|path| path.is_dir()
            && fs::read_to_string(path.join("init.lua")).unwrap() == "old nvim\n"));
    assert!(backups.iter().any(|path| path.is_symlink()
        && path.canonicalize().unwrap() == old_tmux_target.canonicalize().unwrap()));
    assert!(backups
        .iter()
        .any(|path| path.is_file() && fs::read_to_string(path).unwrap() == "old vimrc\n"));

    let second_apply = dot_conf_with_home(&home)
        .arg("--scope")
        .arg("user")
        .arg(&config)
        .output()
        .unwrap();
    assert_success(&second_apply);
    assert_eq!(backup_entries(&home).len(), 3);

    let second_dry_run = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg("--scope")
        .arg("user")
        .arg(&config)
        .output()
        .unwrap();
    assert_success(&second_dry_run);
    let stdout = String::from_utf8_lossy(&second_dry_run.stdout);
    assert_eq!(stdout.matches("[user] unchanged").count(), 4);
    assert!(!stdout.contains("replace"));
}

#[test]
#[serial]
fn multiple_config_files_are_validated_before_any_link_is_applied() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let dotfiles = root.join("dotfiles");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&dotfiles).unwrap();

    write_file(&dotfiles.join(".vimrc"), "set number\n");
    write_file(&dotfiles.join(".gitconfig"), "[user]\n  name = Test\n");

    let shell_config = dotfiles.join("shell.yaml");
    fs::write(
        &shell_config,
        r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
"#,
    )
    .unwrap();

    let git_config = dotfiles.join("git.yaml");
    fs::write(
        &git_config,
        r#"backup_directory: ~/.config/backup
symlinks:
  .gitconfig: ~/.vimrc
"#,
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg(&shell_config)
        .arg(&git_config)
        .output()
        .unwrap();

    assert_failure(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("configured more than once"));
    assert!(!home.join(".vimrc").exists());
    assert!(!home.join(".config/backup").exists());
}
