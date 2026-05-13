use serial_test::serial;
use std::ffi::OsStr;
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

fn with_home<R>(path: &Path, test: impl FnOnce() -> R) -> R {
    let vars: [(&str, Option<&OsStr>); 4] = [
        ("HOME", Some(path.as_os_str())),
        ("USERPROFILE", Some(path.as_os_str())),
        ("HOMEDRIVE", None),
        ("HOMEPATH", None),
    ];
    temp_env::with_vars(vars, test)
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

#[cfg(unix)]
fn create_file_symlink(source: &Path, destination: &Path) {
    std::os::unix::fs::symlink(source, destination).unwrap();
}

#[cfg(windows)]
fn create_file_symlink(source: &Path, destination: &Path) {
    std::os::windows::fs::symlink_file(source, destination).unwrap();
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
    with_home(&home, || {
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
        assert!(!home.join(".config/backup").exists());
    });
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
    with_home(&home, || {
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
    });
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
    with_home(&home, || {
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

        let backup_name = backups[0].file_name().unwrap().to_string_lossy();
        assert!(backup_name.starts_with(".vimrc."));
        assert!(backup_name.ends_with(".bak"));
        assert!(backup_name.contains('T'));
        assert!(backup_name.contains("Z."));
        assert!(!backup_name.contains(':'));
    });
}

#[test]
#[serial]
fn applies_matching_host_links_only() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    with_home(&home, || {
        let hostname = current_test_hostname();
        let matching_host = short_hostname(&hostname);
        let non_matching_host = non_matching_hostname(&hostname);
        write_file(&cfg_dir.join("always"), "always");
        write_file(&cfg_dir.join("work"), "work");
        write_file(&cfg_dir.join("personal"), "personal");

        let yaml = cfg_dir.join("config.yaml");
        fs::write(
            &yaml,
            format!(
                r#"backup_directory: ~/.config/backup
symlinks:
  always: ~/always
  work:
    destinations: ~/work
    host: {matching_host}
  personal:
    destinations:
      - ~/personal
    hosts:
      - {non_matching_host}
"#
            ),
        )
        .unwrap();

        DotConf::from_yaml_file(&yaml)
            .unwrap()
            .apply(Scope::User)
            .unwrap();

        assert!(home.join("always").is_symlink());
        assert!(home.join("work").is_symlink());
        assert!(!home.join("personal").exists());
    });
}

#[test]
#[serial]
fn backs_up_relative_symlink_targets_as_resolved_links() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    let links_dir = home.join("links");
    let targets_dir = home.join("targets");
    fs::create_dir_all(&links_dir).unwrap();
    fs::create_dir_all(&targets_dir).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    with_home(&home, || {
        write_file(&cfg_dir.join(".vimrc"), "new");
        write_file(&targets_dir.join(".vimrc"), "old");
        create_file_symlink(Path::new("../targets/.vimrc"), &links_dir.join(".vimrc"));

        let yaml = cfg_dir.join("config.yaml");
        fs::write(
            &yaml,
            format!(
                "backup_directory: ~/.config/backup\nsymlinks:\n  .vimrc: {}\n",
                links_dir.join(".vimrc").display()
            ),
        )
        .unwrap();

        DotConf::from_yaml_file(&yaml)
            .unwrap()
            .apply(Scope::User)
            .unwrap();

        let backups: Vec<_> = fs::read_dir(home.join(".config/backup"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].is_symlink());
        assert_eq!(
            backups[0].canonicalize().unwrap(),
            targets_dir.join(".vimrc").canonicalize().unwrap()
        );
    });
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
    with_home(&home, || {
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
    });
}

#[test]
#[serial]
fn rejects_unknown_config_keys() {
    let yaml = r#"backup_directory: ~/.config/backup
backup_dir: ~/.config/misspelled
"#;

    let err = DotConf::from_yaml_str(yaml, Path::new("."), Path::new(".")).unwrap_err();
    assert!(format!("{err:#}").contains("unknown field"));
}

#[test]
#[serial]
fn rejects_user_home_tilde_forms() {
    let yaml = "backup_directory: ~other/backup\n";

    let err = DotConf::from_yaml_str(yaml, Path::new("."), Path::new(".")).unwrap_err();

    assert!(format!("{err:#}").contains("unsupported home path"));
}

#[test]
#[serial]
fn resolves_relative_backup_directory_against_config_dir() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    with_home(&home, || {
        write_file(&cfg_dir.join(".vimrc"), "new");
        write_file(&home.join(".vimrc"), "old");

        let yaml = cfg_dir.join("config.yaml");
        fs::write(
            &yaml,
            r#"backup_directory: backups
symlinks:
  .vimrc: ~/.vimrc
"#,
        )
        .unwrap();

        DotConf::from_yaml_file(&yaml)
            .unwrap()
            .apply(Scope::User)
            .unwrap();

        let backups: Vec<_> = fs::read_dir(cfg_dir.join("backups"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "old");
        assert!(!root.join("backups").exists());
    });
}

#[test]
#[serial]
fn resolves_sources_against_canonical_config_path() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let real_cfg_dir = root.join("real-cfg");
    let linked_cfg_dir = root.join("linked-cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&real_cfg_dir).unwrap();
    fs::create_dir_all(&linked_cfg_dir).unwrap();
    with_home(&home, || {
        write_file(&real_cfg_dir.join(".vimrc"), "real");
        write_file(&linked_cfg_dir.join(".vimrc"), "linked");
        let real_yaml = real_cfg_dir.join("config.yaml");
        fs::write(
            &real_yaml,
            r#"backup_directory: ~/.config/backup
symlinks:
  .vimrc: ~/.vimrc
"#,
        )
        .unwrap();
        let linked_yaml = linked_cfg_dir.join("config.yaml");
        create_file_symlink(&real_yaml, &linked_yaml);

        DotConf::from_yaml_file(&linked_yaml)
            .unwrap()
            .apply(Scope::User)
            .unwrap();

        assert_eq!(
            home.join(".vimrc").canonicalize().unwrap(),
            real_cfg_dir.join(".vimrc").canonicalize().unwrap()
        );
    });
}

#[test]
#[serial]
fn backs_up_dangling_symlink_destinations() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let home = root.join("home");
    let cfg_dir = root.join("cfg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&cfg_dir).unwrap();
    with_home(&home, || {
        write_file(&cfg_dir.join(".vimrc"), "new");
        let missing_target = home.join("missing-vimrc");
        create_file_symlink(&missing_target, &home.join(".vimrc"));

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
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].is_symlink());
        assert_eq!(fs::read_link(&backups[0]).unwrap(), missing_target);
    });
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
    with_home(&home, || {
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
    });
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
    with_home(&home, || {
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
    });
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
    with_home(&home, || {
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
        assert!(!home.join(".config/backup").exists());
    });
}
