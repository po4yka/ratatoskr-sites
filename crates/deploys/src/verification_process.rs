//! Bounded, credential-free child processes owned by the verification job.

use std::ffi::OsString;
use std::path::Path;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ProcessError {
    #[error("the verifier is unavailable")]
    Unavailable,
    #[error("verification failed")]
    Rejected,
    #[error("verification exceeded its deadline")]
    Timeout,
    #[error("verification was cancelled")]
    Cancelled,
    #[error("verification output exceeded its bound")]
    TooLarge,
}

/// The only two destinations of tool output: bounded metadata or one private archive file.
#[derive(Clone, Copy)]
pub(crate) enum OutputTarget<'a> {
    Memory(u64),
    File(&'a Path, u64),
}

/// Kill descendants even when a parent future is dropped. The tracked job normally awaits cleanup.
struct GroupGuard(Option<u32>);

impl GroupGuard {
    fn kill(&self) {
        if let Some(raw) = self.0.and_then(|pid| i32::try_from(pid).ok()) {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(raw),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// One fixed executable invocation. No shell, caller environment or credential helper is used.
pub(crate) async fn run(
    executable: &Path,
    arguments: &[OsString],
    directory: &Path,
    deadline: Instant,
    cancel: &CancellationToken,
    target: OutputTarget<'_>,
) -> Result<Vec<u8>, ProcessError> {
    if cancel.is_cancelled() {
        return Err(ProcessError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(ProcessError::Timeout);
    }
    let mut command = command(executable, arguments, directory);
    let mut child = command.spawn().map_err(|_| ProcessError::Unavailable)?;
    let mut group = GroupGuard(child.id());
    let stdout = child.stdout.take().ok_or(ProcessError::Unavailable)?;
    let stderr = child.stderr.take().ok_or(ProcessError::Unavailable)?;
    let result = {
        let work = async {
            let (output, _) = tokio::try_join!(
                drain(stdout, target),
                drain(stderr, OutputTarget::Memory(1024 * 1024))
            )?;
            let status = child.wait().await.map_err(|_| ProcessError::Unavailable)?;
            if status.success() {
                Ok(output)
            } else {
                Err(ProcessError::Rejected)
            }
        };
        tokio::select! {
            biased;
            // cancel-safe: dropping work leaves only unpublished output; the group is killed and reaped below.
            () = cancel.cancelled() => Err(ProcessError::Cancelled),
            () = tokio::time::sleep_until(deadline) => Err(ProcessError::Timeout),
            answer = work => answer,
        }
    };
    group.kill();
    if result.is_err() {
        let _ = child.start_kill();
    }
    // Child::wait is cancel-safe and caches its status after the first successful wait.
    let reaped = child.wait().await.map_err(|_| ProcessError::Unavailable);
    group.0 = None;
    reaped?;
    result
}

fn command(executable: &Path, arguments: &[OsString], directory: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(executable);
    command
        .args(arguments)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GH_CONFIG_DIR", directory.join("gh-config"))
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("XDG_CACHE_HOME", directory.join("cache"))
        .env("XDG_STATE_HOME", directory.join("state"))
        .env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    command.process_group(0);
    command
}

async fn drain(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    target: OutputTarget<'_>,
) -> Result<Vec<u8>, ProcessError> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let (limit, mut file) = match target {
        OutputTarget::Memory(limit) => (limit, None),
        OutputTarget::File(path, limit) => (
            limit,
            Some(
                tokio::fs::File::create_new(path)
                    .await
                    .map_err(|_| ProcessError::Unavailable)?,
            ),
        ),
    };
    let mut output = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|_| ProcessError::Unavailable)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        if total > limit {
            return Err(ProcessError::TooLarge);
        }
        let bytes = buffer.get(..count).ok_or(ProcessError::Unavailable)?;
        if let Some(file) = file.as_mut() {
            // NOT cancel-safe for file bytes, but output is private and the owning job removes it on any refusal.
            file.write_all(bytes)
                .await
                .map_err(|_| ProcessError::Unavailable)?;
        } else {
            output.extend_from_slice(bytes);
        }
    }
    if let Some(file) = file.as_mut() {
        file.flush().await.map_err(|_| ProcessError::Unavailable)?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{OutputTarget, ProcessError, run};
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn oversized_output_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let result = run(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("printf 123456789")],
            directory.path(),
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
            OutputTarget::Memory(8),
        )
        .await;
        assert_eq!(result, Err(ProcessError::TooLarge));
        Ok(())
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::{OutputTarget, ProcessError, run};
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::time::Duration;
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    type Failure = Box<dyn std::error::Error>;

    #[tokio::test]
    async fn file_output_is_streamed_with_a_finite_bound() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let file = directory.path().join("archive");
        let args = [OsString::from("-c"), OsString::from("printf 123456789")];
        let output = run(
            Path::new("/bin/sh"),
            &args,
            directory.path(),
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
            OutputTarget::File(&file, 9),
        )
        .await?;
        assert!(
            output.is_empty(),
            "archive bytes are not captured in memory"
        );
        assert_eq!(std::fs::read(&file)?, b"123456789");
        Ok(())
    }

    #[tokio::test]
    async fn environment_has_only_server_owned_values() -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let output = run(
            Path::new("/usr/bin/env"),
            &[],
            directory.path(),
            Instant::now() + Duration::from_secs(10),
            &CancellationToken::new(),
            OutputTarget::Memory(4096),
        )
        .await?;
        let text = String::from_utf8(output)?;
        let keys: std::collections::BTreeSet<_> = text
            .lines()
            .filter_map(|line| line.split_once('=').map(|(key, _)| key))
            .collect();
        let expected = [
            "PATH",
            "GH_CONFIG_DIR",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "GH_HOST",
            "GH_PROMPT_DISABLED",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            keys, expected,
            "only environment key names are included in a refusal"
        );
        Ok(())
    }

    async fn ready(directory: &Path) -> Result<i32, Failure> {
        Ok(tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(directory.join("ready")).await
                    && let Ok(pid) = text.trim().parse::<i32>()
                {
                    return pid;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?)
    }

    async fn lifecycle(timeout: bool) -> Result<(), Failure> {
        let directory = tempfile::tempdir()?;
        let program = directory.path().join("probe");
        std::fs::write(
            &program,
            b"#!/bin/sh\nsleep 600 &\nprintf '%s' \"$$\" > ready\nwait\n",
        )?;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))?;
        let cancel = CancellationToken::new();
        let owned_cancel = cancel.clone();
        let working = directory.path().to_path_buf();
        let child = tokio::spawn(async move {
            run(
                &program,
                &[],
                &working,
                Instant::now() + Duration::from_secs(1000),
                &owned_cancel,
                OutputTarget::Memory(1024),
            )
            .await
        });
        let pid = match ready(directory.path()).await {
            Ok(pid) => pid,
            Err(error) => {
                cancel.cancel();
                let _ = child.await;
                return Err(error);
            }
        };
        if timeout {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(1001)).await;
            tokio::time::resume();
        } else {
            cancel.cancel();
        }
        let expected = if timeout {
            ProcessError::Timeout
        } else {
            ProcessError::Cancelled
        };
        let result = child.await?;
        assert_eq!(result, Err(expected));
        // The direct child was waited, and no descendant may remain in its process group.
        tokio::time::timeout(Duration::from_secs(10), async {
            while nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn timeout_kills_and_reaps_children() -> Result<(), Failure> {
        lifecycle(true).await
    }

    #[tokio::test]
    async fn cancel_kills_and_reaps_children() -> Result<(), Failure> {
        lifecycle(false).await
    }
}
