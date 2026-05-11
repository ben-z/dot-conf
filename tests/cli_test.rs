use serial_test::serial;
use std::fs;
use std::path::Path;
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

#[test]
#[serial]
fn dry_run_reports_changes_without_mutating_files() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    write_file(&cfg_dir.join(".vimrc"), "new");
    write_file(&home.join(".vimrc"), "old");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
"#,
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg(&yaml)
        .output()
        .unwrap();

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Dry run: no files will be changed."));
    assert!(stdout.contains("[user] replace file"));
    assert!(stdout.contains("backup directory:"));
    assert_eq!(fs::read_to_string(home.join(".vimrc")).unwrap(), "old");
    assert!(!home.join(".config/backup").exists());
}

#[test]
#[serial]
fn missing_source_warning_is_visible_by_default() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        r#"backup_directory: ~/.config/backup
symlinks:
  .missing: ~/.missing
"#,
    )
    .unwrap();

    let output = dot_conf_with_home(&home).arg(&yaml).output().unwrap();

    assert_success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("skipping missing source"));
}

#[test]
#[serial]
fn invalid_later_config_does_not_apply_earlier_config() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    write_file(&cfg_dir.join(".vimrc"), "new");
    let valid_yaml = cfg_dir.join("valid.yaml");
    fs::write(
        &valid_yaml,
        r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
"#,
    )
    .unwrap();
    let invalid_yaml = cfg_dir.join("invalid.yaml");
    fs::write(
        &invalid_yaml,
        r#"backup_directory: ~/.config/backup
backup_dir: ~/.config/misspelled
"#,
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg(&valid_yaml)
        .arg(&invalid_yaml)
        .output()
        .unwrap();

    assert_failure(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown field"));
    assert!(!home.join(".vimrc").exists());
}

#[test]
#[serial]
fn dry_run_fails_when_create_destination_parent_is_invalid() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    write_file(&cfg_dir.join(".vimrc"), "new");
    write_file(&home.join("blocked"), "not a directory");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: ~/.config/backup\nsymlinks:\n  .vimrc: {}\n",
            home.join("blocked/.vimrc").display()
        ),
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg(&yaml)
        .output()
        .unwrap();

    assert_failure(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[user] blocked"));
    assert!(stdout.contains("destination parent:"));
    assert!(!home.join(".config/backup").exists());
}

#[test]
#[serial]
fn dry_run_fails_when_backup_directory_is_invalid() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    write_file(&cfg_dir.join(".vimrc"), "new");
    write_file(&home.join(".vimrc"), "old");
    write_file(&home.join("blocked"), "not a directory");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: {}\nsymlinks:\n  .vimrc: ~/.vimrc\n",
            home.join("blocked/backup").display()
        ),
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg(&yaml)
        .output()
        .unwrap();

    assert_failure(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[user] blocked"));
    assert!(stdout.contains("backup directory:"));
    assert_eq!(fs::read_to_string(home.join(".vimrc")).unwrap(), "old");
}

#[cfg(unix)]
#[test]
#[serial]
fn dry_run_accepts_symlinked_backup_directory() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    let real_backup = home.join("real-backup");
    let backup_link = home.join("backup-link");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::create_dir_all(&real_backup).unwrap();
    std::os::unix::fs::symlink(&real_backup, &backup_link).unwrap();

    write_file(&cfg_dir.join(".vimrc"), "new");
    write_file(&home.join(".vimrc"), "old");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: {}\nsymlinks:\n  .vimrc: ~/.vimrc\n",
            backup_link.display()
        ),
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg(&yaml)
        .output()
        .unwrap();

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[user] replace file"));
    assert!(!stdout.contains("[user] blocked"));
    assert_eq!(fs::read_to_string(home.join(".vimrc")).unwrap(), "old");
}

#[cfg(unix)]
#[test]
#[serial]
fn dry_run_fails_when_backup_directory_lacks_search_permission() {
    use std::os::unix::fs::PermissionsExt;

    if unsafe { libc::geteuid() } == 0 {
        return;
    }

    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    let backup_dir = home.join("backup");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::create_dir_all(&backup_dir).unwrap();
    fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o200)).unwrap();

    write_file(&cfg_dir.join(".vimrc"), "new");
    write_file(&home.join(".vimrc"), "old");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: {}\nsymlinks:\n  .vimrc: ~/.vimrc\n",
            backup_dir.display()
        ),
    )
    .unwrap();

    let output = dot_conf_with_home(&home)
        .arg("--dry-run")
        .arg(&yaml)
        .output()
        .unwrap();
    fs::set_permissions(&backup_dir, fs::Permissions::from_mode(0o700)).unwrap();

    assert_failure(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[user] blocked"));
    assert!(stdout.contains("backup directory:"));
    assert_eq!(fs::read_to_string(home.join(".vimrc")).unwrap(), "old");
}
