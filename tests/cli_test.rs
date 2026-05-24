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

fn current_test_hostname() -> String {
    hostname::get().unwrap().into_string().unwrap()
}

fn short_hostname(hostname: &str) -> &str {
    hostname
        .split_once('.')
        .map_or(hostname, |(short, _)| short)
}

fn non_matching_hostname(hostname: &str) -> String {
    let short = short_hostname(hostname);
    let candidate = "dot-conf-unmatched-host";
    if candidate.eq_ignore_ascii_case(hostname) || candidate.eq_ignore_ascii_case(short) {
        "dot-conf-unmatched-host-2".to_string()
    } else {
        candidate.to_string()
    }
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
fn prints_version_with_long_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_dot-conf"))
        .arg("--version")
        .output()
        .unwrap();

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("dot-conf {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn prints_version_with_short_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_dot-conf"))
        .arg("-V")
        .output()
        .unwrap();

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("dot-conf {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
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
fn dry_run_does_not_report_sudo_for_host_skipped_system_links() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    let non_matching_host = non_matching_hostname(&current_test_hostname());
    write_file(&cfg_dir.join(".vimrc"), "user");
    write_file(&cfg_dir.join(".sysrc"), "sys");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
sys_symlinks:
  .sysrc:
    destinations: /tmp/dot-conf-host-skipped-sysrc
    host: {non_matching_host}
"#
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
    assert!(!stdout.contains("sudo"));
    assert!(stdout.contains("[user] create"));
    assert!(!stdout.contains("[system]"));
}

#[test]
#[serial]
fn all_scope_applies_user_links_without_sudo_when_system_links_are_host_skipped() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();

    let non_matching_host = non_matching_hostname(&current_test_hostname());
    write_file(&cfg_dir.join(".vimrc"), "user");
    write_file(&cfg_dir.join(".sysrc"), "sys");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
sys_symlinks:
  .sysrc:
    destinations: /tmp/dot-conf-host-skipped-sysrc
    host: {non_matching_host}
"#
        ),
    )
    .unwrap();

    let output = dot_conf_with_home(&home).arg(&yaml).output().unwrap();

    assert_success(&output);
    assert!(home.join(".vimrc").is_symlink());
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
