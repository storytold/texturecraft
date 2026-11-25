use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use processkit::{ProcessGroup, ProcessGroupOptions};

use super::{CANCEL_POLL_INTERVAL, EngineError, Result, io};

pub(super) fn run_bounded(command: &Command, stdout: &Path, stderr: &Path, remaining_bytes: u64, canceled: &AtomicBool) -> Result<(bool, String)> {
    // A synchronous engine command may already be running inside a Tokio context
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .spawn_scoped(scope, || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(io)?;
                runtime.block_on(async {
                    let group = ProcessGroup::with_options(ProcessGroupOptions::default().max_memory(remaining_bytes))
                        .map_err(|error| io(format!("could not enforce the MaterialCraft RAM allowance of {remaining_bytes} bytes: {error}")))?;
                    let command =
                        processkit::Command::new(command.get_program()).args(command.get_args()).create_no_window().stdout_file(stdout).stderr_file(stderr);
                    let process = group.start(&command).await.map_err(io)?;
                    let mut wait = std::pin::pin!(process.wait());
                    let cancellation = async {
                        while !canceled.load(Ordering::Relaxed) {
                            tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
                        }
                    };
                    let status = tokio::select! {
                        status = &mut wait => status.map_err(io)?,
                        _ = cancellation => {
                            group.kill_all().map_err(io)?;
                            wait.await.map_err(io)?;
                            return Err(io("MaterialCraft cook canceled"));
                        }
                    };

                    Ok((status.code() == Some(0), format!("{status:?}")))
                })
            })
            .map_err(io)?
            .join()
            .map_err(|_| EngineError::Other("the MaterialCraft process worker stopped".into()))?
    })
}
