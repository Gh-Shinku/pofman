use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use tempfile::TempDir;

struct App {
    dir: TempDir,
    ssh: PathBuf,
}
impl App {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ssh = dir
            .path()
            .join(if cfg!(windows) { "ssh.exe" } else { "ssh" });
        assert!(
            Command::new("rustc")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/ssh.rs"
                ))
                .arg("-o")
                .arg(&ssh)
                .status()
                .unwrap()
                .success()
        );
        Self { dir, ssh }
    }
    fn config(&self, destination: &str) {
        let executable = self.ssh.to_str().unwrap().replace('\\', "/");
        fs::write(
            self.dir.path().join("config.toml"),
            format!("[[profiles]]\nname = 'test'\ncommand = '\"{executable}\" {destination}'\n"),
        )
        .unwrap();
    }
    fn call(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_pofman"))
            .args(args)
            .env("POFMAN_CONFIG", self.dir.path().join("config.toml"))
            .env("POFMAN_STATE_DIR", self.dir.path().join("run"))
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.call(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.call(&["stop", "test"]);
    }
}

#[test]
fn background_lifecycle_and_config_removal() {
    let app = App::new();
    assert!(app.ok(&["list"]).contains("No profiles"));
    app.config("hold");
    assert!(app.ok(&["list"]).contains("stopped"));
    app.config("exit-later");
    app.ok(&["start", "test"]);
    std::thread::sleep(std::time::Duration::from_millis(1600));
    assert!(app.ok(&["list"]).contains("stopped"));
    app.config("hold");
    assert!(app.ok(&["start", "test"]).contains("Started test"));
    assert!(app.ok(&["start", "test"]).contains("already running"));
    let listing = app.ok(&["list"]);
    assert!(listing.contains("running"));
    assert!(listing.contains("PID"));
    assert!(!listing.contains("Config:"));
    assert!(!listing.contains('\u{1b}'));
    fs::write(app.dir.path().join("config.toml"), "").unwrap();
    assert!(app.ok(&["list"]).contains("removed from config"));
    assert!(app.ok(&["stop", "test"]).contains("Stopped test"));
    app.config("hold");
    assert!(app.ok(&["stop", "test"]).contains("already stopped"));
    assert!(!app.call(&["start", "missing"]).status.success());
    assert!(!app.call(&[]).status.success());
    app.config("fail");
    let output = app.call(&["start", "test"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("forwarding failed"));
    assert!(app.ok(&["list"]).contains("stopped"));
    app.config("hold");
    app.ok(&["start", "test"]);
    fs::write(app.dir.path().join("config.toml"), "invalid TOML [").unwrap();
    assert!(app.ok(&["stop", "test"]).contains("Stopped test"));
}
