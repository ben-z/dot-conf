use serial_test::serial;
use std::fs;
use std::path::Path;

use dot_conf::{DotConf, Scope};
use tempfile::tempdir;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn set_home(path: &Path) {
    unsafe {
        std::env::set_var("HOME", path);
        std::env::set_var("USERPROFILE", path);
        std::env::remove_var("HOMEDRIVE");
        std::env::remove_var("HOMEPATH");
    }
}

#[test]
#[serial]
fn loads_and_applies_basic_config() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    set_home(&home);

    write_file(&cfg_dir.join(".vimrc"), "set nu");
    write_file(&cfg_dir.join(".bashrc"), "alias ll='ls -al'");

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
  .bashrc: ~/.bashrc
"#,
    )
    .unwrap();

    let conf = DotConf::from_yaml_file(&yaml).unwrap();
    conf.apply(Scope::User).unwrap();

    assert!(home.join(".vimrc").is_symlink());
    assert_eq!(
        home.join(".vimrc").canonicalize().unwrap(),
        cfg_dir.join(".vimrc").canonicalize().unwrap()
    );
    assert!(home.join(".bashrc").is_symlink());
    assert_eq!(
        fs::read_dir(home.join(".config/backup")).unwrap().count(),
        0
    );
}

#[test]
#[serial]
fn supports_multiple_destinations() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    set_home(&home);

    write_file(&cfg_dir.join(".tmux.conf"), "set -g mouse on");

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        r#"backup_directory: ~/.config/backup
symlinks:
  .tmux.conf:
    - ~/.tmux.conf
    - ~/.config/tmux/tmux.conf
"#,
    )
    .unwrap();

    DotConf::from_yaml_file(&yaml)
        .unwrap()
        .apply(Scope::User)
        .unwrap();

    assert!(home.join(".tmux.conf").is_symlink());
    assert!(home.join(".config/tmux/tmux.conf").is_symlink());
}

#[test]
#[serial]
fn creates_backup_for_existing_files() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    set_home(&home);

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

    DotConf::from_yaml_file(&yaml)
        .unwrap()
        .apply(Scope::User)
        .unwrap();

    let backups: Vec<_> = fs::read_dir(home.join(".config/backup"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "old");
}

#[test]
#[serial]
fn creates_distinct_backups_for_matching_destination_names() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    set_home(&home);

    write_file(&cfg_dir.join("first"), "new-a");
    write_file(&cfg_dir.join("second"), "new-b");
    write_file(&home.join("one/config"), "old-a");
    write_file(&home.join("two/config"), "old-b");

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: ~/.config/backup\nsymlinks:\n  first: {}\n  second: {}\n",
            home.join("one/config").display(),
            home.join("two/config").display()
        ),
    )
    .unwrap();

    DotConf::from_yaml_file(&yaml)
        .unwrap()
        .apply(Scope::User)
        .unwrap();

    let mut backup_contents: Vec<_> = fs::read_dir(home.join(".config/backup"))
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    backup_contents.sort();
    assert_eq!(backup_contents, vec!["old-a", "old-b"]);
}

#[test]
#[serial]
fn rejects_unknown_config_keys() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    set_home(&home);

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        r#"backup_directory: ~/.config/backup
backup_dir: ~/.config/misspelled
"#,
    )
    .unwrap();

    let err = DotConf::from_yaml_file(&yaml).unwrap_err();
    assert!(format!("{err:#}").contains("unknown field"));
}

#[test]
#[serial]
fn all_scope_does_not_apply_user_links_after_system_link_failure() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    let blocked_parent = root.join("blocked");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(&blocked_parent, "not a directory").unwrap();
    set_home(&home);

    write_file(&cfg_dir.join(".vimrc"), "user");
    write_file(&cfg_dir.join(".sysrc"), "sys");

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: ~/.config/backup\nsymlinks:\n  .vimrc: ~/.vimrc\nsys_symlinks:\n  .sysrc: {}\n",
            blocked_parent.join("sysrc").display()
        ),
    )
    .unwrap();

    let err = DotConf::from_yaml_file(&yaml)
        .unwrap()
        .apply(Scope::All)
        .unwrap_err();
    assert!(format!("{err:#}").contains("failed creating"));
    assert!(!home.join(".vimrc").exists());
}

#[test]
#[serial]
fn skips_nonexistent_sources() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    set_home(&home);

    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        r#"backup_directory: ~/.config/backup
symlinks:
  .missing: ~/.missing
"#,
    )
    .unwrap();

    DotConf::from_yaml_file(&yaml)
        .unwrap()
        .apply(Scope::User)
        .unwrap();
    assert!(!home.join(".missing").exists());
}

#[test]
#[serial]
fn applies_system_scope_only() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let etc = root.join("etc");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::create_dir_all(&etc).unwrap();
    set_home(&home);

    write_file(&cfg_dir.join(".sysrc"), "sys");
    let yaml = cfg_dir.join("config.yaml");
    fs::write(
        &yaml,
        format!(
            "backup_directory: ~/.config/backup\nsys_symlinks:\n  .sysrc: {}\n",
            etc.join("sysrc").display()
        ),
    )
    .unwrap();

    let conf = DotConf::from_yaml_file(&yaml).unwrap();
    assert!(conf.requires_root());
    conf.apply(Scope::Sys).unwrap();
    assert!(etc.join("sysrc").is_symlink());
}
