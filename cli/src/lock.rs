// Single-instance lock for `oshihub watch`.
//
// Two watchers polling at once means every go-live notifies twice — the
// exact situation a terminal `oshihub watch` next to the enabled
// oshihub-watch.service produces. The lock makes the second instance refuse
// to start instead.
//
// A plain lockfile (atomic create_new + stored PID) rather than flock():
// flock is the more robust mechanism — the kernel drops it however the
// process dies — but std doesn't expose it, and paying for a crate or an
// unsafe libc call isn't worth it when the failure mode of a stale file is
// a loud refusal that the staleness check below already handles.
//
// The staleness check is NOT just belt-and-braces. `systemctl --user stop`
// sends SIGTERM, which Rust terminates on without unwinding, so Drop never
// runs and the file stays behind. A leftover lock with a dead PID is the
// *normal* aftermath of stopping the service, not a rare crash artifact.

use std::fmt;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// `$XDG_RUNTIME_DIR/oshihub` — tmpfs, wiped at logout/reboot, which is the
/// right lifetime for a lock: a reboot can never leave one behind, and PID
/// reuse across boots (the classic pidfile flaw) can't produce a false
/// "already running".
///
/// The temp_dir fallback only matters off systemd (runtime_dir is `None` on
/// macOS, where watch is academic anyway — notifications shell out to
/// notify-send).
pub fn runtime_dir() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("oshihub")
}

#[derive(Debug)]
pub enum LockError {
    /// Another live process holds the lock.
    AlreadyRunning(u32),
    Io(std::io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::AlreadyRunning(pid) => {
                write!(f, "oshihub watch is already running (pid {pid})")
            }
            LockError::Io(err) => write!(f, "could not create the watch lock: {err}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Held for the lifetime of the watch loop; dropping it releases the lock.
///
/// RAII so every exit path in `watch::run` — Ctrl-C, the deliberate
/// exit on 401 — releases without each one remembering to. SIGTERM skips
/// Drop entirely (see module comment); the staleness check covers that.
#[derive(Debug)]
pub struct WatchLock {
    path: PathBuf,
}

impl Drop for WatchLock {
    fn drop(&mut self) {
        // Nothing useful to do about a failure here: the staleness check
        // makes a leftover file harmless.
        let _ = fs::remove_file(&self.path);
    }
}

pub fn acquire() -> Result<WatchLock, LockError> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir).map_err(LockError::Io)?;
    acquire_at(dir.join("watch.lock"), std::process::id())
}

/// The actual acquire logic, path and PID injected so tests can run against
/// a temp directory and a PID they control.
fn acquire_at(path: PathBuf, own_pid: u32) -> Result<WatchLock, LockError> {
    // Two attempts: the first may find a stale lock and remove it; the
    // retry then re-runs create_new rather than writing over the file, so
    // if two processes steal the same stale lock at once, exactly one wins.
    for _ in 0..2 {
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                // A failed write leaves an empty file, which a later reader
                // treats as stale — wrong direction (it could steal a held
                // lock), but only reachable if the disk breaks between
                // creating a file and writing 7 bytes to it.
                let _ = write!(file, "{own_pid}");
                return Ok(WatchLock { path });
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                match holder_pid(&path) {
                    Some(pid) if pid_is_alive(pid) => {
                        return Err(LockError::AlreadyRunning(pid));
                    }
                    // Dead PID, or unreadable/garbage contents (a crashed
                    // half-write): stale either way. Refusing on garbage
                    // would wedge watch until someone hand-deletes the file.
                    _ => {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
            Err(err) => return Err(LockError::Io(err)),
        }
    }

    // Lost the steal race on the retry — whoever won is the running watcher.
    match holder_pid(&path) {
        Some(pid) => Err(LockError::AlreadyRunning(pid)),
        None => Err(LockError::Io(std::io::Error::other(
            "lock contended while being stolen",
        ))),
    }
}

fn holder_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

// ---- TUI presence marks ----
//
// NOT a lock: any number of TUIs may run at once, and none of them may ever
// block watch from *running*. Each TUI drops a per-PID mark; watch checks
// "is any TUI open?" before popping a notification, on the theory that a
// dashboard already on screen makes a popup about it redundant. Per-PID
// files rather than one shared mark because a second TUI's exit must not
// un-silence watch while the first is still open.

/// Held for the TUI's lifetime; dropping it removes the mark. Same SIGTERM
/// caveat as `WatchLock` — the stale-PID sweep in `tui_is_present` is the
/// cleanup path when Drop never ran.
#[derive(Debug)]
pub struct TuiPresence {
    path: PathBuf,
}

impl Drop for TuiPresence {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Mark this process as a running TUI. `None` on failure, deliberately
/// swallowed: presence is a courtesy signal to watch, and the TUI must
/// never refuse to start over it.
pub fn mark_tui_present() -> Option<TuiPresence> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir).ok()?;
    mark_tui_present_in(&dir, std::process::id())
}

fn mark_tui_present_in(dir: &Path, pid: u32) -> Option<TuiPresence> {
    let path = dir.join(format!("tui.{pid}.presence"));
    fs::write(&path, "").ok()?;
    Some(TuiPresence { path })
}

/// Is any TUI open right now? Also sweeps marks whose PID is dead, so a
/// SIGKILLed TUI can't silence watch forever.
pub fn tui_is_present() -> bool {
    tui_is_present_in(&runtime_dir())
}

fn tui_is_present_in(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let mut present = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = presence_pid(&name.to_string_lossy()) else {
            continue; // watch.lock, or anything else sharing the dir
        };
        if pid_is_alive(pid) {
            present = true;
        } else {
            let _ = fs::remove_file(entry.path());
        }
    }
    present
}

fn presence_pid(file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix("tui.")?
        .strip_suffix(".presence")?
        .parse()
        .ok()
}

/// Liveness via /proc, which is Linux-only — fine here, since watch's whole
/// output channel (notify-send → mako) already is. Checking existence, not
/// readability: /proc/{pid} is world-readable for any live process.
fn pid_is_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique file per test — the suite runs multi-threaded in one process,
    /// so each test keys its path on its own name and starts clean.
    fn test_path(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "oshihub-lock-test-{}-{name}.lock",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    /// A PID that cannot belong to a live process: the kernel's pid_max tops
    /// out at 2^22 (4194304) even when raised to its maximum, so
    /// /proc/4294967295 can never exist.
    const DEAD_PID: u32 = u32::MAX;

    #[test]
    fn acquire_writes_own_pid_into_a_fresh_lock() {
        let path = test_path("fresh");
        let _lock = acquire_at(path.clone(), 4242).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "4242");
    }

    #[test]
    fn refuses_while_the_holder_is_alive() {
        let path = test_path("held");
        // The test process itself is the one PID guaranteed to be alive.
        let holder = std::process::id();
        fs::write(&path, holder.to_string()).unwrap();

        match acquire_at(path.clone(), 4242) {
            Err(LockError::AlreadyRunning(pid)) => assert_eq!(pid, holder),
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        // Refusing must not disturb the holder's lock.
        assert_eq!(fs::read_to_string(&path).unwrap(), holder.to_string());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn steals_a_lock_whose_holder_is_dead() {
        let path = test_path("stale");
        fs::write(&path, DEAD_PID.to_string()).unwrap();

        let _lock = acquire_at(path.clone(), 4242).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "4242");
    }

    // A crashed half-write leaves garbage or nothing; both must read as
    // stale, or watch wedges until someone hand-deletes the file.
    #[test]
    fn steals_a_lock_with_garbage_contents() {
        let path = test_path("garbage");
        fs::write(&path, "not a pid").unwrap();
        assert!(acquire_at(path, 4242).is_ok());
    }

    #[test]
    fn steals_an_empty_lock() {
        let path = test_path("empty");
        fs::write(&path, "").unwrap();
        assert!(acquire_at(path, 4242).is_ok());
    }

    #[test]
    fn drop_releases_the_lock() {
        let path = test_path("drop");
        let lock = acquire_at(path.clone(), 4242).unwrap();
        assert!(path.exists());
        drop(lock);
        assert!(!path.exists());
    }

    #[test]
    fn can_reacquire_after_release() {
        let path = test_path("reacquire");
        drop(acquire_at(path.clone(), 4242).unwrap());
        assert!(acquire_at(path, 4243).is_ok());
    }

    #[test]
    fn unwritable_directory_reports_io_not_already_running() {
        let path = test_path("no-such-dir/watch");
        match acquire_at(path, 4242) {
            Err(LockError::Io(_)) => {}
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    /// Fresh directory per test — presence checks scan a whole dir, so
    /// sharing one across parallel tests would cross-contaminate.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oshihub-presence-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn presence_pid_parses_only_the_expected_shape() {
        assert_eq!(presence_pid("tui.1234.presence"), Some(1234));
        assert_eq!(presence_pid("watch.lock"), None);
        assert_eq!(presence_pid("tui.abc.presence"), None);
        assert_eq!(presence_pid("tui.1234"), None);
        assert_eq!(presence_pid("1234.presence"), None);
    }

    #[test]
    fn live_mark_means_present() {
        let dir = test_dir("live-mark");
        // The test process is the one PID guaranteed alive.
        let _mark = mark_tui_present_in(&dir, std::process::id()).unwrap();
        assert!(tui_is_present_in(&dir));
    }

    #[test]
    fn no_marks_means_absent() {
        let dir = test_dir("no-marks");
        assert!(!tui_is_present_in(&dir));
    }

    #[test]
    fn missing_directory_means_absent() {
        assert!(!tui_is_present_in(Path::new("/no/such/dir")));
    }

    #[test]
    fn dead_mark_is_absent_and_swept() {
        let dir = test_dir("dead-mark");
        let stale = dir.join(format!("tui.{DEAD_PID}.presence"));
        fs::write(&stale, "").unwrap();

        assert!(!tui_is_present_in(&dir));
        // The sweep must have deleted it, so a SIGKILLed TUI can't
        // silence watch forever.
        assert!(!stale.exists());
    }

    // The two-TUIs scenario the per-PID scheme exists for: one TUI gone
    // (its mark stale) must not hide the one still open — and the sweep
    // must remove only the stale mark, not the live one.
    #[test]
    fn a_stale_mark_next_to_a_live_one_still_reads_present() {
        let dir = test_dir("two-tuis");
        let _live = mark_tui_present_in(&dir, std::process::id()).unwrap();
        let stale = dir.join(format!("tui.{DEAD_PID}.presence"));
        fs::write(&stale, "").unwrap();

        assert!(tui_is_present_in(&dir));
        assert!(!stale.exists());
        assert!(tui_is_present_in(&dir));
    }

    #[test]
    fn dropping_the_mark_removes_it() {
        let dir = test_dir("drop-mark");
        let mark = mark_tui_present_in(&dir, std::process::id()).unwrap();
        assert!(tui_is_present_in(&dir));
        drop(mark);
        assert!(!tui_is_present_in(&dir));
    }

    // A presence mark and the watch lock share the directory; neither may
    // confuse the other's file for its own.
    #[test]
    fn watch_lock_and_presence_marks_coexist() {
        let dir = test_dir("coexist");
        let _lock = acquire_at(dir.join("watch.lock"), std::process::id()).unwrap();
        assert!(!tui_is_present_in(&dir));

        let _mark = mark_tui_present_in(&dir, std::process::id()).unwrap();
        assert!(tui_is_present_in(&dir));
        // And the sweep didn't eat the lock.
        assert!(dir.join("watch.lock").exists());
    }
}
