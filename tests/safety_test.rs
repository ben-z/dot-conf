use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn config(
        &self,
        name: &str,
        backup: &Path,
        user: &[(&Path, &Path)],
        system: &[(&Path, &Path)],
    ) -> PathBuf {
        #[derive(Serialize)]
        struct Config<'a> {
            backup_directory: &'a Path,
            symlinks: BTreeMap<&'a Path, &'a Path>,
            sys_symlinks: BTreeMap<&'a Path, &'a Path>,
        }
        self.write(
            name,
            &serde_saphyr::to_string(&Config {
                backup_directory: backup,
                symlinks: user.iter().copied().collect(),
                sys_symlinks: system.iter().copied().collect(),
            })
            .unwrap(),
        )
    }

    fn run(&self, configs: &[&Path], args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dot-conf"))
            .current_dir(self.dir.path())
            .env("HOME", self.path("home"))
            .env("USERPROFILE", self.path("home"))
            .env_remove("RUST_LOG")
            .args(args)
            .args(configs)
            .output()
            .unwrap()
    }

    fn rejects_without_changes(&self, configs: &[&Path], reason: &str) {
        let before = snapshot(self.dir.path());
        for args in [&["--dry-run"][..], &[][..]] {
            let output = self.run(configs, args);
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.status.success(), "unexpected success: {text}");
            assert!(
                text.to_ascii_lowercase()
                    .contains(&reason.to_ascii_lowercase()),
                "expected {reason:?}: {text}"
            );
            assert_eq!(
                snapshot(self.dir.path()),
                before,
                "failed command changed filesystem"
            );
        }
    }
}

// Inspect entries without following symlinks; include directory creation and raw
// link targets so failures cannot hide a backup or leave a broken replacement.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (String, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, (String, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let (kind, contents) = if metadata.file_type().is_symlink() {
                (
                    "symlink",
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                )
            } else if metadata.is_dir() {
                visit(root, &path, entries);
                ("directory", Vec::new())
            } else if metadata.is_file() {
                ("file", fs::read(&path).unwrap())
            } else {
                ("special", Vec::new())
            };
            entries.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                (kind.into(), contents),
            );
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[test]
fn self_mapping_is_rejected_by_preview_and_apply() {
    let f = Fixture::new();
    let source = f.write("source", "keep me");
    let config = f.config(
        "config.yaml",
        &f.path("backups"),
        &[(&source, &source)],
        &[],
    );
    f.rejects_without_changes(&[&config], "source and destination are the same path");
}

#[test]
fn replacing_a_directory_cannot_move_its_source_or_backups() {
    for protected in ["source", "backups"] {
        let f = Fixture::new();
        let destination = f.path("destination");
        fs::create_dir(&destination).unwrap();
        let source = f.write(
            if protected == "source" {
                "destination/source"
            } else {
                "source"
            },
            "new",
        );
        let backup = f.path(if protected == "backups" {
            "destination/backups"
        } else {
            "backups"
        });
        let config = f.config("config.yaml", &backup, &[(&source, &destination)], &[]);
        f.rejects_without_changes(&[&config], "would move or retarget");
    }
}

#[test]
fn destinations_cannot_overlap() {
    let f = Fixture::new();
    let first = f.write("source1", "one");
    let second = f.write("source2", "two");
    let config = f.config(
        "config.yaml",
        &f.path("backups"),
        &[
            (&first, &f.path("new-directory")),
            (&second, &f.path("new-directory/child")),
        ],
        &[],
    );
    f.rejects_without_changes(&[&config], "overlapping destinations");
}

#[test]
fn parent_components_cannot_hide_duplicate_destinations_across_configs() {
    let f = Fixture::new();
    fs::create_dir(f.path("sub")).unwrap();
    let first = f.write("source1", "one");
    let second = f.write("source2", "two");
    let a = f.config(
        "a.yaml",
        &f.path("backups"),
        &[(&first, &f.path("target"))],
        &[],
    );
    let b = f.config(
        "b.yaml",
        &f.path("backups"),
        &[(&second, &f.path("sub/../target"))],
        &[],
    );
    f.rejects_without_changes(&[&a, &b], "configured more than once");
}

#[cfg(unix)]
#[test]
fn symlinked_parents_cannot_hide_duplicate_destinations() {
    let f = Fixture::new();
    fs::create_dir(f.path("real")).unwrap();
    std::os::unix::fs::symlink(f.path("real"), f.path("alias")).unwrap();
    let first = f.write("source1", "one");
    let second = f.write("source2", "two");
    let config = f.config(
        "config.yaml",
        &f.path("backups"),
        &[
            (&first, &f.path("real/target")),
            (&second, &f.path("alias/target")),
        ],
        &[],
    );
    f.rejects_without_changes(&[&config], "configured more than once");
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_hide_source_or_backup_containment() {
    for protected in ["source", "backups"] {
        let f = Fixture::new();
        fs::create_dir_all(f.path("destination/backups")).unwrap();
        let source = f.write("source", "new");
        let target = if protected == "source" {
            f.write("destination/source", "keep")
        } else {
            f.path("destination/backups")
        };
        std::os::unix::fs::symlink(target, f.path("alias")).unwrap();
        let backup = if protected == "backups" {
            f.path("alias")
        } else {
            f.path("backups")
        };
        let source = if protected == "source" {
            f.path("alias")
        } else {
            source
        };
        let config = f.config(
            "config.yaml",
            &backup,
            &[(&source, &f.path("destination"))],
            &[],
        );
        f.rejects_without_changes(&[&config], "would move or retarget");
    }
}

#[test]
fn later_filesystem_blockers_are_found_before_any_config_changes() {
    let f = Fixture::new();
    let source = f.write("source", "new");
    f.write("target", "old");
    f.write("not-a-directory", "keep");
    let a = f.config(
        "a.yaml",
        &f.path("backups"),
        &[(&source, &f.path("target"))],
        &[],
    );
    let b = f.config(
        "b.yaml",
        &f.path("backups"),
        &[],
        &[(&source, &f.path("not-a-directory/target"))],
    );
    f.rejects_without_changes(&[&a, &b], "not-a-directory");
}

#[test]
fn later_missing_sources_fail_before_any_config_changes() {
    let f = Fixture::new();
    let source = f.write("source", "new");
    f.write("target", "old");
    let a = f.config(
        "a.yaml",
        &f.path("backups"),
        &[(&source, &f.path("target"))],
        &[],
    );
    let b = f.config(
        "b.yaml",
        &f.path("backups"),
        &[(&f.path("missing"), &f.path("new-target"))],
        &[],
    );
    f.rejects_without_changes(&[&a, &b], "source missing");
}

#[test]
fn default_run_applies_both_scopes_quietly_without_sudo() {
    let f = Fixture::new();
    let source = f.write("source with spaces", "new");
    let config = f.config(
        "config.yaml",
        &f.path("backups"),
        &[(&source, &f.path("user"))],
        &[(&source, &f.path("system"))],
    );
    let output = f.run(&[&config], &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    for destination in ["user", "system"] {
        assert_eq!(fs::read_to_string(f.path(destination)).unwrap(), "new");
        assert!(fs::symlink_metadata(f.path(destination))
            .unwrap()
            .file_type()
            .is_symlink());
    }
    assert!(!f.path("backups").exists());
    assert!(f.run(&[&config], &[]).status.success());
    assert!(!f.path("backups").exists());
}

#[test]
fn empty_paths_destinations_and_host_names_fail() {
    for entry in [
        "source: []",
        "source: ''",
        "'': target",
        "source: {destinations: target, hosts: ['']}",
    ] {
        let f = Fixture::new();
        let config = f.write(
            "config.yaml",
            &format!("backup_directory: backups\nsymlinks:\n  {entry}\n"),
        );
        f.rejects_without_changes(
            &[&config],
            if entry.ends_with("[]") {
                "no destinations"
            } else if entry.contains("hosts:") {
                "empty hostname"
            } else {
                "path must not be empty"
            },
        );
    }
}

#[cfg(unix)]
#[test]
fn a_closed_output_pipe_returns_an_error_without_panicking_or_changing_files() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;
    let f = Fixture::new();
    let source = f.write("source", "new");
    let config = f.config(
        "config.yaml",
        &f.path("backups"),
        &[(&source, &f.path("target"))],
        &[],
    );
    let before = snapshot(f.dir.path());
    let (writer, reader) = UnixStream::pair().unwrap();
    drop(reader);
    let output = Command::new(env!("CARGO_BIN_EXE_dot-conf"))
        .arg("--dry-run")
        .arg(config)
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(stderr.contains("broken pipe"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(snapshot(f.dir.path()), before);
}

#[test]
fn a_directory_cannot_be_linked_inside_itself() {
    let f = Fixture::new();
    let source = f.path("source");
    fs::create_dir(&source).unwrap();
    let config = f.config(
        "config.yaml",
        &f.path("backups"),
        &[(&source, &source.join("recursive"))],
        &[],
    );
    f.rejects_without_changes(&[&config], "inside source directory");
}

#[cfg(windows)]
#[test]
fn windows_ambiguous_final_names_are_rejected() {
    for name in ["short~1", "trailing.", "trailing "] {
        let f = Fixture::new();
        let source = f.write("source", "new");
        let config = f.config(
            "config.yaml",
            &f.path("backups"),
            &[(&source, &f.path(name))],
            &[],
        );
        f.rejects_without_changes(&[&config], "Windows paths must use long filenames");
    }
}
