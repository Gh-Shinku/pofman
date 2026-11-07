use crate::config::{Config, Paths};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, Process, ProcessStatus, ProcessesToUpdate, System};

pub struct Manager {
    pub paths: Paths,
}

pub struct Row {
    pub name: String,
    pub command: String,
    pub pid: Option<u32>,
}
impl Row {
    pub fn status(&self) -> &'static str {
        if self.pid.is_some() {
            "running"
        } else {
            "stopped"
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    name: String,
    command: String,
    pid: u32,
    started: u64,
    exe: PathBuf,
}

struct ObservedProcess {
    system: System,
    pid: Pid,
}

impl std::ops::Deref for ObservedProcess {
    type Target = Process;
    fn deref(&self) -> &Process {
        // The immutable snapshot retains this process for its entire lifetime.
        self.system.process(self.pid).expect("observed process")
    }
}

fn process(pid: u32) -> Result<Option<ObservedProcess>> {
    let mut system = System::new().context("Cannot inspect processes")?;
    let pid = Pid::from_u32(pid);
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    Ok(system
        .process(pid)
        .is_some()
        .then_some(ObservedProcess { system, pid }))
}

impl Record {
    fn matches(&self, process: &Process) -> bool {
        process.start_time() == self.started
            && process.exe() == Some(self.exe.as_path())
            && !matches!(
                process.status(),
                ProcessStatus::Zombie | ProcessStatus::Dead
            )
    }
}

impl Manager {
    pub fn new(paths: Paths) -> Result<Self> {
        paths.ensure_config()?;
        fs::create_dir_all(&paths.state).context("Cannot create process state directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&paths.state, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { paths })
    }

    fn lock(&self) -> Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.paths.state.join("manager.lock"))?;
        file.try_lock_exclusive()
            .context("Another pofman operation is in progress; retry shortly")?;
        Ok(file)
    }

    fn records(&self) -> Result<Vec<Record>> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.paths.state)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let record: Record = toml::from_str(&fs::read_to_string(&path)?)
                .with_context(|| format!("Invalid process state: {}", path.display()))?;
            if process(record.pid)?.is_some_and(|p| record.matches(&p)) {
                records.push(record);
            } else {
                fs::remove_file(path)?;
            }
        }
        Ok(records)
    }

    pub fn rows(&self) -> Result<Vec<Row>> {
        let _lock = self.lock()?;
        let config = Config::load(&self.paths)?;
        let mut records = self.records()?;
        let mut rows = Vec::new();
        for profile in config.profiles {
            let pid = records
                .iter()
                .position(|r| r.name == profile.name)
                .map(|i| records.remove(i).pid);
            rows.push(Row {
                name: profile.name,
                command: profile.command,
                pid,
            });
        }
        for record in records {
            rows.push(Row {
                name: record.name,
                command: format!("[removed from config] {}", record.command),
                pid: Some(record.pid),
            });
        }
        Ok(rows)
    }

    pub fn start(&self, name: &str) -> Result<String> {
        let _lock = self.lock()?;
        if let Some(record) = self.records()?.iter().find(|r| r.name == name) {
            return Ok(format!("{name} already running (PID {})", record.pid));
        }
        let config = Config::load(&self.paths)?;
        let profile = config
            .profiles
            .iter()
            .find(|p| p.name == name)
            .with_context(|| {
                format!(
                    "Unknown profile: {name}; config: {}",
                    self.paths.config.display()
                )
            })?;
        let args = profile.argv()?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let log = self.paths.state.join(format!("ssh-{stamp}.log"));
        let stderr = OpenOptions::new().write(true).create_new(true).open(&log)?;
        let mut command = Command::new(&args[0]);
        command
            .args([
                "-N",
                "-T",
                "-oBatchMode=yes",
                "-oExitOnForwardFailure=yes",
                "-oForkAfterAuthentication=no",
                "-oControlMaster=no",
                "-oControlPath=none",
                "-oServerAliveInterval=30",
                "-oServerAliveCountMax=3",
            ])
            .args(&args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
        detach(&mut command)?;
        let mut child = command
            .spawn()
            .context("Cannot start ssh; ensure OpenSSH is installed and on PATH")?;
        let pid = child.id();
        let result = (|| -> Result<String> {
            thread::sleep(Duration::from_millis(500));
            if let Some(status) = child.try_wait()? {
                let details = fs::read_to_string(&log).unwrap_or_default();
                bail!(
                    "SSH exited ({status}): {}\nLog: {}",
                    details.trim(),
                    log.display()
                );
            }
            let info = process(pid)?.context("SSH exited during startup")?;
            let record = Record {
                name: name.into(),
                command: profile.command.clone(),
                pid,
                started: info.start_time(),
                exe: info
                    .exe()
                    .context("Cannot identify SSH executable")?
                    .to_path_buf(),
            };
            let state = self.paths.state.join(format!("{pid}.toml"));
            let pending = self.paths.state.join(format!("{pid}.pending"));
            fs::write(&pending, toml::to_string(&record)?)?;
            fs::rename(pending, state)?;
            Ok(format!(
                "Started {name} (PID {pid}); process running, connection may still be establishing. Log: {}",
                log.display()
            ))
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(unix)]
        if result.is_ok() {
            // Reap the child if the interactive parent remains alive.
            thread::spawn(move || {
                let _ = child.wait();
            });
        }
        result
    }

    pub fn stop(&self, name: &str) -> Result<String> {
        let _lock = self.lock()?;
        let records = self.records()?;
        let Some(record) = records.iter().find(|r| r.name == name) else {
            if !Config::load(&self.paths)?
                .profiles
                .iter()
                .any(|p| p.name == name)
            {
                bail!("Unknown profile: {name}");
            }
            return Ok(format!("{name} already stopped"));
        };
        if let Some(info) = process(record.pid)?
            && record.matches(&info)
        {
            if !info.kill() {
                bail!("Cannot stop {name} (PID {})", record.pid);
            }
            for _ in 0..40 {
                if !process(record.pid)?.is_some_and(|p| record.matches(&p)) {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            if process(record.pid)?.is_some_and(|p| record.matches(&p)) {
                bail!("SSH has not exited; retry stop");
            }
        }
        fs::remove_file(self.paths.state.join(format!("{}.toml", record.pid)))?;
        Ok(format!("Stopped {name}"))
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    // Rust's Windows spawn inherits inheritable handles beyond the explicit stdio.
    // A captured pofman stdout/stderr would otherwise remain open in background SSH,
    // leaving scripts waiting for EOF even after pofman itself exits. This process
    // only spawns SSH, whose stdio is explicitly redirected above.
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: only operating on this process's existing standard handles;
        // changing inheritance does not close them or affect ongoing I/O.
        unsafe {
            let handle = GetStdHandle(kind);
            if !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) == 0
            {
                return Err(std::io::Error::last_os_error())
                    .context("Cannot detach SSH standard handles");
            }
        }
    }
    command.creation_flags(0x08000000 | 0x00000200); // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_pid_does_not_stop_an_unrelated_live_process() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(Paths {
            config: dir.path().join("config.toml"),
            state: dir.path().join("run"),
        })
        .unwrap();
        fs::write(
            &manager.paths.config,
            "[[profiles]]\nname='test'\ncommand='ssh host'\n",
        )
        .unwrap();
        let pid = std::process::id();
        let current = process(pid).unwrap().unwrap();
        let record = Record {
            name: "test".into(),
            command: "ssh host".into(),
            pid,
            started: current.start_time() + 1,
            exe: current.exe().unwrap().to_path_buf(),
        };
        let path = manager.paths.state.join(format!("{pid}.toml"));
        fs::write(&path, toml::to_string(&record).unwrap()).unwrap();
        assert!(manager.stop("test").unwrap().contains("already stopped"));
        assert!(!path.exists());
        assert!(process(pid).unwrap().is_some());
    }
}

#[cfg(unix)]
fn detach(command: &mut Command) -> Result<()> {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and this closure touches no shared state.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}
