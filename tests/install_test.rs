#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};

struct Installer {
    root: TempDir,
    archive: String,
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

impl Installer {
    fn new(target: &str) -> Self {
        let root = tempdir().unwrap();
        for directory in ["tools", "download", "payload", "temporary", "bin"] {
            fs::create_dir(root.path().join(directory)).unwrap();
        }
        fs::copy(
            env!("CARGO_BIN_EXE_dot-conf"),
            root.path().join("payload/dot-conf"),
        )
        .unwrap();
        let archive = format!("dot-conf-{target}.tar.gz");
        executable(
            &root.path().join("tools/uname"),
            "#!/bin/sh\ncase \"$1\" in -s) echo \"$TEST_OS\";; -m) echo \"$TEST_ARCH\";; *) exit 1;; esac\n",
        );
        executable(
            &root.path().join("tools/curl"),
            r#"#!/bin/sh
set -eu
[ "$#" = 4 ] && [ "$1" = -fsSL ] && [ "$3" = -o ]
printf '%s\n' "$2" >> "$DOWNLOAD_LOG"
name="${2##*/}"
if [ "$name" = "${FAIL_ASSET:-}" ]; then
  echo "fixture download failed: $name" >&2
  exit 22
fi
cp "$DOWNLOAD_FIXTURES/$name" "$4"
"#,
        );
        executable(
            &root.path().join("bin/dot-conf"),
            "#!/bin/sh\necho original\n",
        );
        let installer = Self { root, archive };
        installer.package();
        installer
    }

    fn package(&self) {
        let output = Command::new("tar")
            .arg("-czf")
            .arg(self.root.path().join("download").join(&self.archive))
            .arg("-C")
            .arg(self.root.path().join("payload"))
            .arg("dot-conf")
            .output()
            .unwrap();
        assert_success(&output);
        let digest = Command::new("bash")
            .arg("-c")
            .arg("if command -v sha256sum >/dev/null; then sha256sum \"$1\"; else shasum -a 256 \"$1\"; fi")
            .arg("checksum")
            .arg(&self.archive)
            .current_dir(self.root.path().join("download"))
            .output()
            .unwrap();
        assert_success(&digest);
        fs::write(
            self.root
                .path()
                .join("download")
                .join(format!("{}.sha256", self.archive)),
            digest.stdout,
        )
        .unwrap();
    }

    fn command(&self, os: &str, arch: &str) -> Command {
        let mut paths = vec![self.root.path().join("tools")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let mut command = Command::new("bash");
        command
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh"))
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("TMPDIR", self.root.path().join("temporary"))
            .env("DOT_CONF_INSTALL_DIR", self.root.path().join("bin"))
            .env("DOWNLOAD_FIXTURES", self.root.path().join("download"))
            .env("DOWNLOAD_LOG", self.root.path().join("downloads.log"))
            .env("TEST_OS", os)
            .env("TEST_ARCH", arch)
            .env_remove("FAIL_ASSET");
        command
    }

    fn installed_binary(&self) -> PathBuf {
        self.root.path().join("bin/dot-conf")
    }

    fn assert_clean(&self) {
        assert_eq!(
            fs::read_dir(self.root.path().join("temporary"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read_dir(self.root.path().join("bin")).unwrap().count(),
            1
        );
    }

    fn assert_preserved(&self, output: &Output) {
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(self.installed_binary()).unwrap(),
            "#!/bin/sh\necho original\n"
        );
        self.assert_clean();
    }
}

#[test]
fn installs_verified_binary_for_each_supported_platform() {
    for (os, arch, target) in [
        ("Linux", "x86_64", "x86_64-unknown-linux-musl"),
        ("Linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("Darwin", "x86_64", "x86_64-apple-darwin"),
        ("Darwin", "arm64", "aarch64-apple-darwin"),
    ] {
        let installer = Installer::new(target);
        assert_success(&installer.command(os, arch).output().unwrap());
        let output = Command::new(installer.installed_binary())
            .arg("--version")
            .output()
            .unwrap();
        assert_success(&output);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("dot-conf {}\n", env!("CARGO_PKG_VERSION"))
        );
        let root = installer.root.path();
        fs::write(root.join("source"), "new config").unwrap();
        fs::write(root.join("destination"), "old config").unwrap();
        fs::write(
            root.join("config.yaml"),
            "backup_directory: backup\nsymlinks:\n  source: destination\n",
        )
        .unwrap();
        let preview = Command::new(installer.installed_binary())
            .args(["--dry-run", "config.yaml"])
            .current_dir(root)
            .output()
            .unwrap();
        assert_success(&preview);
        assert_eq!(
            fs::read_to_string(root.join("destination")).unwrap(),
            "old config"
        );
        assert!(!root.join("backup").exists());
        for _ in 0..2 {
            let output = Command::new(installer.installed_binary())
                .arg("config.yaml")
                .current_dir(root)
                .output()
                .unwrap();
            assert_success(&output);
            assert_eq!(
                root.join("destination").canonicalize().unwrap(),
                root.join("source").canonicalize().unwrap()
            );
            let backups: Vec<_> = fs::read_dir(root.join("backup"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(backups.len(), 1);
            assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "old config");
        }
        installer.assert_clean();
    }
}

#[test]
fn failed_downloads_preserve_installed_binary_without_trying_other_targets() {
    for checksum in [false, true] {
        let installer = Installer::new("x86_64-unknown-linux-musl");
        let asset = if checksum {
            format!("{}.sha256", installer.archive)
        } else {
            installer.archive.clone()
        };
        let output = installer
            .command("Linux", "x86_64")
            .env("FAIL_ASSET", asset)
            .output()
            .unwrap();
        installer.assert_preserved(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("fixture download failed"));
        let downloads = fs::read_to_string(installer.root.path().join("downloads.log")).unwrap();
        assert_eq!(downloads.lines().count(), if checksum { 2 } else { 1 });
        assert!(!downloads.contains("linux-gnu"));
    }
}

#[test]
fn tampered_archive_preserves_installed_binary() {
    let installer = Installer::new("x86_64-unknown-linux-musl");
    fs::write(
        installer
            .root
            .path()
            .join("download")
            .join(&installer.archive),
        "tampered",
    )
    .unwrap();
    let output = installer.command("Linux", "x86_64").output().unwrap();
    installer.assert_preserved(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("FAILED"));
}

#[test]
fn unsupported_platform_fails_before_download() {
    let installer = Installer::new("x86_64-unknown-linux-musl");
    let output = installer.command("Linux", "riscv64").output().unwrap();
    installer.assert_preserved(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unsupported OS/architecture"));
    assert!(!installer.root.path().join("downloads.log").exists());
}

#[test]
fn non_executable_payload_preserves_installed_binary() {
    let installer = Installer::new("x86_64-unknown-linux-musl");
    fs::set_permissions(
        installer.root.path().join("payload/dot-conf"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    installer.package();
    let output = installer.command("Linux", "x86_64").output().unwrap();
    installer.assert_preserved(&output);
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("Archive must contain an executable file named dot-conf"));
}

#[test]
fn default_install_directory_is_under_home() {
    let installer = Installer::new("x86_64-unknown-linux-musl");
    let output = installer
        .command("Linux", "x86_64")
        .env_remove("DOT_CONF_INSTALL_DIR")
        .env("HOME", installer.root.path().join("home"))
        .output()
        .unwrap();
    assert_success(&output);
    let installed = installer.root.path().join("home/.local/bin/dot-conf");
    let output = Command::new(installed).arg("--version").output().unwrap();
    assert_success(&output);
    installer.assert_clean();
}
