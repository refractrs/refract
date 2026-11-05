//! Own child processes so interrupted builds and dev sessions leave no app running.
use crate::Result;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

pub struct ManagedChild(Child);

impl ManagedChild {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        Ok(Self(command.spawn()?))
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        // Every managed child starts a new process group with its PID as PGID.
        // Kill the group even when the parent exited, to also reap its workers.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = Command::new("taskkill")
                .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Drain both streams concurrently, forwarding diagnostics while Cargo works.
pub fn capture(
    command: &mut Command,
    stopped: &AtomicBool,
    cargo_json: bool,
    quiet: bool,
) -> Result<Output> {
    if stopped.load(Ordering::Relaxed) {
        return Err("interrupted".into());
    }
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let mut child = ManagedChild::spawn(command)?;
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let out_sender = sender.clone();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if out_sender.send((false, line)).is_err() {
                break;
            }
        }
    });
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr)
            .lines()
            .map_while(std::result::Result::ok)
        {
            if sender.send((true, line)).is_err() {
                break;
            }
        }
    });
    let mut output = Output {
        success: false,
        stdout: String::new(),
        stderr: String::new(),
    };
    loop {
        if stopped.load(Ordering::Relaxed) {
            drop(child);
            let _ = out_thread.join();
            let _ = err_thread.join();
            return Err("interrupted".into());
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok((is_stderr, line)) => {
                let destination = if is_stderr {
                    &mut output.stderr
                } else {
                    &mut output.stdout
                };
                destination.push_str(&line);
                destination.push('\n');
                if !quiet {
                    if is_stderr || !cargo_json {
                        eprintln!("{line}");
                    } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                        if let Some(rendered) = value["message"]["rendered"].as_str() {
                            eprint!("{rendered}");
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    while child.try_wait()?.is_none() {
        if stopped.load(Ordering::Relaxed) {
            return Err("interrupted".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    output.success = child.try_wait()?.is_some_and(|status| status.success());
    let _ = out_thread.join();
    let _ = err_thread.join();
    Ok(output)
}

pub fn checked(command: &mut Command, stopped: &AtomicBool) -> Result<()> {
    let output = capture(command, stopped, false, false)?;
    if !output.success {
        return Err(format!("command failed: {command:?}").into());
    }
    Ok(())
}
