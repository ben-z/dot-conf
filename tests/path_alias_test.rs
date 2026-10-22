use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{tempdir, TempDir};

struct Fixture(TempDir);

impl Fixture {
    fn new() -> Self {
        Self(tempdir().unwrap())
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    fn write(&self, name: &str, contents: &str) {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn config(&self, name: &str, backup: &str, links: &[(&str, &str)]) {
        #[derive(Serialize)]
        struct Config<'a> {
            backup_directory: &'a str,
            symlinks: BTreeMap<&'a str, &'a str>,
        }
        self.write(
            name,
            &serde_saphyr::to_string(&Config {
                backup_directory: backup,
                symlinks: links.iter().copied().collect(),
            })
            .unwrap(),
        );
    }

    fn rejects_without_changes(&self, configs: &[&str], reason: &str) {
        let before = snapshot(self.0.path());
        for args in [&["--dry-run"][..], &[][..]] {
            let output = Command::new(env!("CARGO_BIN_EXE_dot-conf"))
                .current_dir(self.0.path())
                .env("HOME", self.path("home"))
                .env("USERPROFILE", self.path("home"))
                .env_remove("RUST_LOG")
                .args(args)
                .args(configs)
                .output()
                .unwrap();
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.status.success(), "unexpected success: {text}");
            assert!(text.contains(reason), "expected {reason:?}: {text}");
            assert_eq!(snapshot(self.0.path()), before, "command changed files");
            assert!(!self.path("backups").exists());
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Entry {
    File(Vec<u8>),
    Directory,
    Symlink(PathBuf),
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Entry>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let state = if metadata.file_type().is_symlink() {
                Entry::Symlink(fs::read_link(&path).unwrap())
            } else if metadata.is_dir() {
                visit(root, &path, entries);
                Entry::Directory
            } else {
                Entry::File(fs::read(&path).unwrap())
            };
            entries.insert(path.strip_prefix(root).unwrap().to_path_buf(), state);
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[cfg(any(target_os = "macos", windows))]
const PATH_ALIASES: [(&str, &str); 5] = [
    ("source", "SOURCE"),
    ("café", "cafe\u{301}"),
    ("S", "ſ"),
    ("Σ", "ς"),
    ("ß", "ss"),
];

#[cfg(any(target_os = "macos", windows))]
#[test]
fn case_and_unicode_aliases_cannot_replace_their_source() {
    for (source, destination) in PATH_ALIASES {
        let f = Fixture::new();
        f.write(source, "preserve source");
        f.config("config.yaml", "backups", &[(source, destination)]);
        f.rejects_without_changes(&["config.yaml"], "source and destination are the same path");
    }
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn absent_case_and_unicode_destination_aliases_are_rejected_across_configs() {
    for (first, second) in PATH_ALIASES {
        let f = Fixture::new();
        f.write("input1", "one");
        f.write("input2", "two");
        f.config("a.yaml", "backups", &[("input1", first)]);
        f.config("b.yaml", "backups", &[("input2", second)]);
        f.rejects_without_changes(&["a.yaml", "b.yaml"], "configured more than once");
    }
}

#[cfg(unix)]
#[test]
fn replacing_a_symlink_parent_cannot_retarget_another_source() {
    let f = Fixture::new();
    f.write("old/config", "old source");
    f.write("a-new/config", "new source");
    std::os::unix::fs::symlink("old", f.path("alias")).unwrap();
    f.config(
        "config.yaml",
        "backups",
        &[("a-new", "alias"), ("alias/config", "target")],
    );
    f.rejects_without_changes(&["config.yaml"], "would move or retarget source");
}

#[cfg(unix)]
#[test]
fn replacing_a_symlink_parent_cannot_retarget_the_backup_directory() {
    let f = Fixture::new();
    f.write("old/keep", "preserve old directory");
    f.write("a-new/keep", "preserve new directory");
    f.write("source", "new file");
    f.write("target", "old file");
    std::os::unix::fs::symlink("old", f.path("alias")).unwrap();
    f.config(
        "config.yaml",
        "alias/saved",
        &[("a-new", "alias"), ("source", "target")],
    );
    f.rejects_without_changes(&["config.yaml"], "would move or retarget backup directory");
}

#[cfg(unix)]
#[test]
fn replacing_a_link_in_a_source_symlink_chain_is_rejected() {
    let f = Fixture::new();
    f.write("old-source", "preserve original source");
    f.write("a-new-source", "new source");
    std::os::unix::fs::symlink("middle", f.path("source-alias")).unwrap();
    std::os::unix::fs::symlink("old-source", f.path("middle")).unwrap();
    f.config(
        "config.yaml",
        "backups",
        &[("a-new-source", "middle"), ("source-alias", "target")],
    );
    f.rejects_without_changes(&["config.yaml"], "would move or retarget source");
}

#[cfg(unix)]
#[test]
fn replacing_a_link_in_a_backup_symlink_chain_is_rejected() {
    let f = Fixture::new();
    f.write("old/keep", "preserve old directory");
    f.write("a-new/keep", "preserve new directory");
    f.write("source", "new file");
    f.write("target", "old file");
    std::os::unix::fs::symlink("middle", f.path("backup-alias")).unwrap();
    std::os::unix::fs::symlink("old", f.path("middle")).unwrap();
    f.config(
        "config.yaml",
        "backup-alias",
        &[("a-new", "middle"), ("source", "target")],
    );
    f.rejects_without_changes(&["config.yaml"], "would move or retarget backup directory");
}

#[cfg(unix)]
#[test]
fn parent_components_in_a_symlink_cycle_do_not_evade_cycle_detection() {
    let f = Fixture::new();
    fs::create_dir(f.path("sub")).unwrap();
    std::os::unix::fs::symlink("sub/../source", f.path("source")).unwrap();
    f.config("config.yaml", "backups", &[("source", "target")]);
    f.rejects_without_changes(&["config.yaml"], "failed inspecting source");
}

#[test]
fn explicit_null_host_filters_are_rejected() {
    for field in ["host", "hosts"] {
        for null in ["null", "~", ""] {
            let f = Fixture::new();
            f.write("source", "new file");
            f.write(
                "config.yaml",
                &format!(
                    "backup_directory: backups\nsymlinks:\n  source:\n    destinations: target\n    {field}: {null}\n"
                ),
            );
            f.rejects_without_changes(&["config.yaml"], "failed parsing YAML");
        }
    }
}
