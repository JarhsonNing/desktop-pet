use crate::{
    config::atomic_json,
    platform::{self, DesktopSnapshot},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Lease {
    pub version: u8,
    pub identity: String,
    pub parent_pid: u32,
    pub deadline: u64,
    pub snapshot: DesktopSnapshot,
}

pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn lock(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive()?;
    Ok(file)
}

fn desktop_lock(directory: &Path) -> io::Result<File> {
    lock(&directory.join("desktop.lock"))
}
pub fn lease_path(directory: &Path) -> PathBuf {
    directory.join("desktop-lease.json")
}

trait DesktopOps {
    fn snapshot(&self) -> io::Result<DesktopSnapshot>;
    fn hide(&self) -> io::Result<()>;
    fn restore(&self, snapshot: &DesktopSnapshot) -> io::Result<()>;
}

struct NativeDesktop;
impl DesktopOps for NativeDesktop {
    fn snapshot(&self) -> io::Result<DesktopSnapshot> {
        platform::snapshot()
    }
    fn hide(&self) -> io::Result<()> {
        platform::hide_icons()
    }
    fn restore(&self, snapshot: &DesktopSnapshot) -> io::Result<()> {
        platform::restore_icons(snapshot)
    }
}

pub fn read_lease(directory: &Path) -> io::Result<Option<Lease>> {
    match fs::read(lease_path(directory)) {
        Ok(bytes) => {
            let lease: Lease = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if lease.version != 1 {
                return Err(io::Error::other("Unknown desktop recovery journal version"));
            }
            platform::validate_snapshot(&lease.snapshot)?;
            Ok(Some(lease))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn restore(directory: &Path) -> io::Result<bool> {
    let _lock = desktop_lock(directory)?;
    restore_locked(directory, &NativeDesktop)
}

fn restore_locked(directory: &Path, backend: &impl DesktopOps) -> io::Result<bool> {
    restore_checked(directory, backend, None)
}

fn restore_checked(
    directory: &Path,
    backend: &impl DesktopOps,
    identity: Option<&str>,
) -> io::Result<bool> {
    let Some(lease) = read_lease(directory)? else {
        return Ok(false);
    };
    if identity.is_some_and(|expected| expected != lease.identity) {
        return Ok(false);
    }
    backend.restore(&lease.snapshot)?;
    fs::remove_file(lease_path(directory))?;
    Ok(true)
}

pub fn hide(directory: &Path, seconds: u64) -> io::Result<Child> {
    prepare_hide(directory, seconds, &NativeDesktop, |identity| {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--desktop-guard")
            .arg(identity)
            .arg("--data-dir")
            .arg(directory)
            .spawn()?;
        thread::sleep(Duration::from_millis(100));
        if child.try_wait()?.is_some() {
            return Err(io::Error::other(
                "Recovery watchdog did not start; desktop left untouched",
            ));
        }
        Ok(child)
    })
}

fn prepare_hide<Guard>(
    directory: &Path,
    seconds: u64,
    backend: &impl DesktopOps,
    start_guard: impl FnOnce(&str) -> io::Result<Guard>,
) -> io::Result<Guard> {
    let _lock = desktop_lock(directory)?;
    if read_lease(directory)?.is_some() {
        return Err(io::Error::other(
            "A desktop recovery lease is already active",
        ));
    }
    let lease = Lease {
        version: 1,
        parent_pid: std::process::id(),
        identity: format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ),
        deadline: timestamp() + seconds.clamp(5, 30),
        snapshot: backend.snapshot()?,
    };
    atomic_json(&lease_path(directory), &lease)?;
    let watchdog = match start_guard(&lease.identity) {
        Ok(child) => child,
        Err(error) => {
            fs::remove_file(lease_path(directory))?;
            return Err(error);
        }
    };
    if let Err(error) = backend.hide() {
        let rollback = restore_locked(directory, backend);
        return Err(io::Error::other(format!("{error}; rollback: {rollback:?}")));
    }
    Ok(watchdog)
}

pub fn guard(directory: &Path, identity: &str) -> io::Result<()> {
    let started = Instant::now();
    loop {
        let Some(lease) = read_lease(directory)? else {
            return Ok(());
        };
        if lease.identity != identity {
            return Ok(());
        }
        if timestamp() >= lease.deadline
            || !platform::parent_alive(lease.parent_pid)
            || started.elapsed() >= Duration::from_secs(35)
        {
            let mut last_error = None;
            for _ in 0..20 {
                let guarded_restore = desktop_lock(directory)
                    .and_then(|_lock| restore_checked(directory, &NativeDesktop, Some(identity)));
                match guarded_restore {
                    Ok(_) => return Ok(()),
                    Err(error) => {
                        last_error = Some(error);
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
            return Err(last_error.unwrap_or_else(|| io::Error::other("Recovery failed")));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mochi-{name}-{}-{}",
            std::process::id(),
            timestamp()
        ))
    }
    #[test]
    fn settings_write_and_lock_are_exclusive() {
        let directory = temporary("lock");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        atomic_json(&path, &crate::config::Config::default()).unwrap();
        assert_eq!(crate::config::load(&path).unwrap().name, "麻薯");
        let first = lock(&directory.join("instance.lock")).unwrap();
        assert!(lock(&directory.join("instance.lock")).is_err());
        drop(first);
        assert!(lock(&directory.join("instance.lock")).is_ok());
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn missing_lease_never_touches_desktop() {
        let directory = temporary("empty");
        assert!(!restore(&directory).unwrap());
        guard(&directory, "none").unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn malformed_lease_is_retained_for_manual_recovery() {
        let directory = temporary("invalid");
        fs::create_dir_all(&directory).unwrap();
        fs::write(lease_path(&directory), b"invalid").unwrap();
        assert!(read_lease(&directory).is_err());
        assert!(lease_path(&directory).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    mod recovery {
        use super::*;
        use std::cell::Cell;

        struct FakeDesktop {
            visible: Cell<bool>,
            fail_hide: bool,
            fail_restore: bool,
        }
        impl DesktopOps for FakeDesktop {
            fn snapshot(&self) -> io::Result<DesktopSnapshot> {
                if !self.visible.get() {
                    return Err(io::Error::other("already hidden"));
                }
                #[cfg(target_os = "macos")]
                {
                    Ok(DesktopSnapshot::Mac { previous: None })
                }
                #[cfg(target_os = "windows")]
                {
                    Ok(DesktopSnapshot::Windows { previous: true })
                }
            }
            fn hide(&self) -> io::Result<()> {
                self.visible.set(false);
                if self.fail_hide {
                    Err(io::Error::other("hide failed after mutation"))
                } else {
                    Ok(())
                }
            }
            fn restore(&self, _snapshot: &DesktopSnapshot) -> io::Result<()> {
                if self.fail_restore {
                    Err(io::Error::other("restore failed"))
                } else {
                    self.visible.set(true);
                    Ok(())
                }
            }
        }
        fn backend() -> FakeDesktop {
            FakeDesktop {
                visible: Cell::new(true),
                fail_hide: false,
                fail_restore: false,
            }
        }

        #[test]
        fn journal_precedes_mutation_and_guard_failure_leaves_desktop_untouched() {
            let directory = temporary("guard-failure");
            let desktop = backend();
            let result: io::Result<()> = prepare_hide(&directory, 15, &desktop, |_| {
                assert!(lease_path(&directory).exists());
                assert!(desktop.visible.get());
                Err(io::Error::other("guard failed"))
            });
            assert!(result.is_err());
            assert!(desktop.visible.get());
            assert!(!lease_path(&directory).exists());
            fs::remove_dir_all(directory).unwrap();
        }
        #[test]
        fn partial_hide_failure_rolls_back() {
            let directory = temporary("rollback");
            let mut desktop = backend();
            desktop.fail_hide = true;
            assert!(prepare_hide(&directory, 15, &desktop, |_| Ok(())).is_err());
            assert!(desktop.visible.get());
            assert!(!lease_path(&directory).exists());
            fs::remove_dir_all(directory).unwrap();
        }
        #[test]
        fn failed_restore_keeps_recovery_journal() {
            let directory = temporary("restore-failure");
            let mut desktop = backend();
            prepare_hide(&directory, 1000, &desktop, |_| Ok(())).unwrap();
            let lease = read_lease(&directory).unwrap().unwrap();
            assert!(lease.deadline <= timestamp() + 30);
            assert!(!desktop.visible.get());
            desktop.fail_restore = true;
            assert!(restore_locked(&directory, &desktop).is_err());
            assert!(lease_path(&directory).exists());
            desktop.fail_restore = false;
            assert!(restore_locked(&directory, &desktop).unwrap());
            assert!(desktop.visible.get());
            assert!(!lease_path(&directory).exists());
            fs::remove_dir_all(directory).unwrap();
        }
        #[test]
        fn a_stale_guard_never_restores_a_new_lease() {
            let directory = temporary("stale");
            let desktop = backend();
            prepare_hide(&directory, 15, &desktop, |_| Ok(())).unwrap();
            guard(&directory, "an-old-lease").unwrap();
            assert!(lease_path(&directory).exists());
            assert!(!desktop.visible.get());
            assert!(!restore_checked(&directory, &desktop, Some("an-old-lease")).unwrap());
            assert!(!desktop.visible.get());
            restore_locked(&directory, &desktop).unwrap();
            fs::remove_dir_all(directory).unwrap();
        }
        #[test]
        fn already_hidden_desktop_and_active_lease_are_preserved() {
            let directory = temporary("preserve");
            let desktop = backend();
            desktop.visible.set(false);
            assert!(prepare_hide(&directory, 15, &desktop, |_| Ok(())).is_err());
            assert!(!lease_path(&directory).exists());
            desktop.visible.set(true);
            prepare_hide(&directory, 15, &desktop, |_| Ok(())).unwrap();
            let identity = read_lease(&directory).unwrap().unwrap().identity;
            assert!(prepare_hide(&directory, 15, &desktop, |_| Ok(())).is_err());
            assert_eq!(read_lease(&directory).unwrap().unwrap().identity, identity);
            restore_locked(&directory, &desktop).unwrap();
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
