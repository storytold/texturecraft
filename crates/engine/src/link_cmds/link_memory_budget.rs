use crate::{EngineError, Result};

/// Budget shared by retained link data and all phases of one cook.
#[derive(Clone, Copy, Debug)]
pub struct LinkMemoryBudget {
    cap_bytes: u64,
    retained_bytes: u64,
}

impl LinkMemoryBudget {
    pub fn new(cap_bytes: u64) -> Self {
        Self { cap_bytes, retained_bytes: 0 }
    }

    pub(crate) fn reserve(&mut self, bytes: u64) -> Result<()> {
        self.retained_bytes = self.checked_total(bytes)?;
        Ok(())
    }

    pub(crate) fn check(self, additional_bytes: u64) -> Result<()> {
        self.checked_total(additional_bytes).map(|_| ())
    }

    fn checked_total(self, additional_bytes: u64) -> Result<u64> {
        let required_bytes = u128::from(self.retained_bytes) + u128::from(additional_bytes);
        if required_bytes > u128::from(self.cap_bytes) {
            return Err(EngineError::Other(format!(
                "MaterialCraft cook requires an estimated {required_bytes} bytes of RAM, exceeding the configured cap of {} bytes",
                self.cap_bytes
            )));
        }

        Ok(required_bytes as u64)
    }

    pub(super) fn set_retained(&mut self, bytes: u64) {
        self.retained_bytes = bytes;
    }

    pub(super) fn remaining(self) -> u64 {
        self.cap_bytes.saturating_sub(self.retained_bytes)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn child_allowance(self) -> u64 {
        let allowance = self.remaining().min(texturecraft_formats::available_save_resources().1);
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
        let allowance = allowance.min(rustix::process::getrlimit(rustix::process::Resource::As).current.unwrap_or(u64::MAX));

        allowance
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn check_available(self, temporary_bytes: u64) -> Result<()> {
        let available_bytes = texturecraft_formats::available_save_resources().1;
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
        let available_bytes = match rustix::process::getrlimit(rustix::process::Resource::As).current {
            // Physical memory can be plentiful while the host has little address space left
            Some(limit) => available_bytes.min(limit.saturating_sub(host_virtual_bytes()?)),
            None => available_bytes,
        };

        if temporary_bytes > available_bytes {
            return Err(EngineError::Other(format!(
                "MaterialCraft cook requires an estimated {temporary_bytes} additional bytes of RAM, with {available_bytes} bytes currently available and a configured cap of {} bytes",
                self.cap_bytes
            )));
        }

        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn host_virtual_bytes() -> Result<u64> {
    let status = std::fs::read_to_string("/proc/self/status").map_err(|error| EngineError::Other(format!("cannot check host address space: {error}")))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmSize:"))
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .and_then(|kib| kib.checked_mul(1024))
        .ok_or_else(|| EngineError::Other("cannot determine the memory available under the host address-space limit".into()))
}

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn host_virtual_bytes() -> Result<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new().map_err(|error| EngineError::Other(format!("cannot check host address space: {error}")))?;
    let pid = Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, ProcessRefreshKind::nothing().with_memory());
    system
        .process(pid)
        .map(|process| process.virtual_memory())
        .filter(|bytes| *bytes != 0)
        .ok_or_else(|| EngineError::Other("cannot determine the memory available under the host address-space limit".into()))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn inherited_address_limit_restricts_host_and_companion() {
        const PROCESS_LIMIT_BYTES: u64 = 192 << 20;
        const CONSTRAINED_TEST: &str = "TC_LINK_TEST_ADDRESS_LIMIT";

        // Isolate the inherited limit from unrelated tests in the parent process
        if std::env::var_os(CONSTRAINED_TEST).is_none() {
            let status = std::process::Command::new("prlimit")
                .arg(format!("--as={PROCESS_LIMIT_BYTES}"))
                .arg("--")
                .arg(std::env::current_exe().unwrap())
                .args(["--exact", std::thread::current().name().unwrap()])
                .env(CONSTRAINED_TEST, "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }

        let budget = LinkMemoryBudget::new(8 << 30);
        let error = budget.check_available(PROCESS_LIMIT_BYTES).unwrap_err();

        assert!(error.to_string().contains("configured cap"));
        assert!(budget.child_allowance() <= PROCESS_LIMIT_BYTES);
        budget.check_available(1).unwrap();
    }
}
