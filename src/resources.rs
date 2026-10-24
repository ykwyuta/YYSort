//! OS enforced limits apply only to the supervised worker, never to the caller.
use crate::{Result, error};
use std::sync::atomic::{AtomicBool, Ordering};
pub static CANCELLED: AtomicBool = AtomicBool::new(false);
#[derive(serde::Serialize)]
pub struct Observed {
    pub affinity_cpus: u32,
    pub memory_ceiling_bytes: u64,
    pub memory_ceiling_kind: &'static str,
    pub peak_resident_bytes: u64,
}
impl Guard {
    pub fn observed(&self) -> Result<Observed> {
        #[cfg(target_os = "linux")]
        unsafe {
            let mut cpus: libc::cpu_set_t = std::mem::zeroed();
            if libc::sched_getaffinity(0, std::mem::size_of_val(&cpus), &mut cpus) != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut limit: libc::rlimit = std::mem::zeroed();
            let mut usage: libc::rusage = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_AS, &mut limit) != 0
                || libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Observed {
                affinity_cpus: u32::try_from(libc::CPU_COUNT(&cpus))?,
                memory_ceiling_bytes: limit.rlim_cur,
                memory_ceiling_kind: "virtual_address_space",
                peak_resident_bytes: usage.ru_maxrss as u64 * 1024,
            })
        }
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::{JobObjects::*, ProcessStatus::*, Threading::*};
            let mut allowed = 0usize;
            let mut system = 0usize;
            GetProcessAffinityMask(GetCurrentProcess(), &mut allowed, &mut system)?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            QueryInformationJobObject(
                Some(self.0),
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of_val(&info) as u32,
                None,
            )?;
            let mut memory = PROCESS_MEMORY_COUNTERS::default();
            memory.cb = std::mem::size_of_val(&memory) as u32;
            if !K32GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, memory.cb).as_bool() {
                return Err(windows::core::Error::from_thread().into());
            }
            Ok(Observed {
                affinity_cpus: allowed.count_ones(),
                memory_ceiling_bytes: info.ProcessMemoryLimit as u64,
                memory_ceiling_kind: "committed_virtual_memory",
                peak_resident_bytes: memory.PeakWorkingSetSize as u64,
            })
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            Err(error("unsupported resource monitoring platform"))
        }
    }
}
pub fn check() -> Result<()> {
    if CANCELLED.load(Ordering::Relaxed) {
        Err(error("cancelled"))
    } else {
        Ok(())
    }
}
pub fn install_interrupt() -> Result<()> {
    #[cfg(unix)]
    unsafe {
        extern "C" fn handler(_: libc::c_int) {
            CANCELLED.store(true, Ordering::Relaxed);
        }
        libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
    }
    #[cfg(windows)]
    unsafe {
        use windows::{Win32::System::Console::*, core::BOOL};
        unsafe extern "system" fn handler(event: u32) -> BOOL {
            if matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
                CANCELLED.store(true, Ordering::Relaxed);
                true.into()
            } else {
                false.into()
            }
        }
        SetConsoleCtrlHandler(Some(handler), true)?;
    }
    Ok(())
}
#[cfg(windows)]
pub struct Guard(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(unix)]
pub struct Guard;

pub fn apply(memory: u64, cores: usize, cpu_seconds: Option<u64>) -> Result<Guard> {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut available: libc::cpu_set_t = std::mem::zeroed();
        if libc::sched_getaffinity(0, std::mem::size_of_val(&available), &mut available) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut selected: libc::cpu_set_t = std::mem::zeroed();
        let mut count = 0;
        for i in 0..libc::CPU_SETSIZE as usize {
            if libc::CPU_ISSET(i, &available) && count < cores {
                libc::CPU_SET(i, &mut selected);
                count += 1;
            }
        }
        if count < cores {
            return Err(error(format!(
                "--threads {cores} exceeds available CPUs {count}"
            )));
        }
        if libc::sched_setaffinity(0, std::mem::size_of_val(&selected), &selected) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        for (resource, value) in [
            (libc::RLIMIT_AS, Some(memory)),
            (libc::RLIMIT_CPU, cpu_seconds),
            (libc::RLIMIT_CORE, Some(0)),
        ] {
            if let Some(value) = value {
                let mut old: libc::rlimit = std::mem::zeroed();
                if libc::getrlimit(resource, &mut old) != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let limit = libc::rlimit {
                    rlim_cur: value.min(old.rlim_cur),
                    rlim_max: old.rlim_max,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
        }
        Ok(Guard)
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::{JobObjects::*, Threading::*},
        };
        let process = GetCurrentProcess();
        let mut allowed = 0usize;
        let mut system = 0usize;
        GetProcessAffinityMask(process, &mut allowed, &mut system)?;
        let mut selected = 0usize;
        let mut count = 0;
        for i in 0..usize::BITS {
            let bit = 1usize << i;
            if allowed & bit != 0 && count < cores {
                selected |= bit;
                count += 1;
            }
        }
        if count < cores {
            return Err(error(format!(
                "--threads {cores} exceeds available CPUs {count} in this processor group"
            )));
        }
        SetProcessAffinityMask(process, selected)?;
        let handle = CreateJobObjectW(None, None)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        info.ProcessMemoryLimit = usize::try_from(memory)?;
        if let Some(seconds) = cpu_seconds {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
            info.BasicLimitInformation.PerProcessUserTimeLimit = i64::try_from(
                seconds
                    .checked_mul(10_000_000)
                    .ok_or_else(|| error("CPU time limit too large"))?,
            )?;
        }
        let result = (|| -> windows::core::Result<()> {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )?;
            AssignProcessToJobObject(handle, process)
        })();
        if let Err(e) = result {
            let _ = CloseHandle(handle);
            return Err(e.into());
        }
        Ok(Guard(handle))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (memory, cores, cpu_seconds);
        Err(error("resource limits are supported on Windows and Linux"))
    }
}
