use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tracing::{debug, debug_span, warn};
use tracing_futures::Instrument;

use crate::core::Config;
use crate::ui::{Spinner, Status};

// TODO(#125): Do what is documented here, take a look at what cargo-util does.
/// Replaces the current process with the target process.
///
/// On Unix, this executes the process using the Unix syscall `execvp`, which will block this
/// process, and will only return if there is an error.
///
/// On Windows this isn't technically possible. Instead we emulate it to the best of our ability.
/// One aspect we fix here is that we specify a handler for the Ctrl-C handler.
/// In doing so (and by effectively ignoring it) we should emulate proxying Ctrl-C handling to
/// the application at hand, which will either terminate or handle it itself.
/// According to Microsoft's documentation at
/// <https://docs.microsoft.com/en-us/windows/console/ctrl-c-and-ctrl-break-signals>.
/// the Ctrl-C signal is sent to all processes attached to a terminal, which should include our
/// child process. If the child terminates then we'll reap them in Cargo pretty quickly, and if
/// the child handles the signal then we won't terminate (and we shouldn't!) until the process
/// itself later exits.
#[tracing::instrument(level = "debug")]
pub fn exec_replace(cmd: &mut Command) -> Result<()> {
    let exit_status = cmd
        .spawn()
        .with_context(|| format!("failed to spawn: {}", cmd.get_program().to_string_lossy()))?
        .wait()
        .with_context(|| {
            format!(
                "failed to wait for process to finish: {}",
                cmd.get_program().to_string_lossy()
            )
        })?;

    if exit_status.success() {
        Ok(())
    } else {
        bail!("process did not exit successfully: {exit_status}");
    }
}

/// Runs the process, waiting for completion, and mapping non-success exit codes to an error.
#[tracing::instrument(level = "trace", skip_all)]
pub async fn async_exec(cmd: &mut tokio::process::Command, config: &Config) -> Result<()> {
    let cmd_str = shlex_join(cmd.as_std());

    config.ui().verbose(Status::new("Running", &cmd_str));
    let _spinner = config.ui().widget(Spinner::new(cmd_str.clone()));

    async fn pipe_to_logs<T: AsyncRead + Unpin>(stream: T) {
        let mut reader = BufReader::new(stream).lines();
        loop {
            let line = reader.next_line().await;
            match line {
                Ok(Some(line)) => debug!("{line}"),
                Ok(None) => break,
                Err(err) => warn!("{err:?}"),
            }
        }
    }
    let runtime = config.runtime_handle();
    let mut proc = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| anyhow!("could not execute process: {cmd_str}"))?;

    let span = Arc::new(debug_span!("exec", pid = proc.id()));
    let _enter = span.enter();
    debug!("{cmd_str}");

    let stdout = proc.stdout.take().expect("we asked Rust to pipe stdout");
    let span = debug_span!("out");
    runtime.spawn(async move {
        let mut stdout = stdout;
        pipe_to_logs(&mut stdout).instrument(span).await;
    });

    let stderr = proc.stderr.take().expect("we asked Rust to pipe stderr");
    runtime.spawn(async move {
        let span = debug_span!("err");
        let mut stderr = stderr;
        pipe_to_logs(&mut stderr).instrument(span).await
    });

    let exit_status = proc
        .wait()
        .await
        .with_context(|| anyhow!("could not wait for proces termination: {cmd_str}"))?;
    if exit_status.success() {
        Ok(())
    } else {
        bail!("process did not exit successfully: {exit_status}");
    }
}

#[cfg(unix)]
pub fn is_executable<P: AsRef<Path>>(path: P) -> bool {
    use std::fs;
    use std::os::unix::prelude::*;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn is_executable<P: AsRef<Path>>(path: P) -> bool {
    path.as_ref().is_file()
}

/// Python's [`shlex.join`] for [`Command`].
///
/// [`shlex.join`]: https://docs.python.org/3/library/shlex.html#shlex.join
fn shlex_join(cmd: &Command) -> String {
    ShlexJoin(cmd).to_string()
}

struct ShlexJoin<'a>(&'a Command);

impl<'a> fmt::Display for ShlexJoin<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn write_quoted(f: &mut fmt::Formatter<'_>, s: &OsStr) -> fmt::Result {
            let utf = s.to_string_lossy();
            if utf.contains('"') {
                write!(f, "{s:?}")
            } else {
                write!(f, "{utf}")
            }
        }

        let cmd = &self.0;
        write_quoted(f, cmd.get_program())?;

        for arg in cmd.get_args() {
            write!(f, " ")?;
            write_quoted(f, arg)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
pub fn make_executable(path: &Path) {
    use std::fs;
    use std::os::unix::prelude::*;
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(perms.mode() | 0o700);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(windows)]
pub fn make_executable(_path: &Path) {}
