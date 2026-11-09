//! The resources of a run: CPU slots and the memory budget of the evaluation.

use std::sync::{Mutex, OnceLock, PoisonError};

use sekai_pipeline::Options;
use sekai_pipeline::sched::Resident;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::export::BLOCK_CACHE_BYTES;

/// Parses a byte count: a whole number with an optional binary suffix `K`, `M`, `G` or `T`, such
/// as `512M` or `8G`.
pub(crate) fn parse_bytes(text: &str) -> Result<u64, String> {
    let invalid = || format!("invalid byte count `{text}`: expected a number such as 512M or 8G");
    let (digits, shift) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => {
            let shift = match c.to_ascii_uppercase() {
                'K' => 10,
                'M' => 20,
                'G' => 30,
                'T' => 40,
                _ => return Err(invalid()),
            };
            (&text[..i], shift)
        }
        _ => (text, 0),
    };
    let value: u64 = digits.parse().map_err(|_| invalid())?;
    value.checked_mul(1 << shift).ok_or_else(invalid)
}

/// The system information of this process, kept between reads.
fn system() -> Option<&'static Mutex<(System, Pid)>> {
    static SYSTEM: OnceLock<Option<Mutex<(System, Pid)>>> = OnceLock::new();
    SYSTEM
        .get_or_init(|| {
            let pid = sysinfo::get_current_pid().ok()?;
            Some(Mutex::new((System::new().ok()?, pid)))
        })
        .as_ref()
}

/// The resident memory of this process, or 0 when it cannot be read.
fn resident() -> u64 {
    let Some(system) = system() else {
        return 0;
    };
    let mut guard = system.lock().unwrap_or_else(PoisonError::into_inner);
    let (system, pid) = &mut *guard;
    let kind = ProcessRefreshKind::nothing().with_memory();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[*pid]), true, kind);
    system.process(*pid).map_or(0, |process| process.memory())
}

/// The physical memory of the machine and the resident memory of this process, when known.
fn memory_now() -> Option<(u64, u64)> {
    let total = {
        let mut guard = system()?.lock().unwrap_or_else(PoisonError::into_inner);
        guard.0.refresh_memory();
        guard.0.total_memory()
    };
    Some((total, resident()))
}

/// The memory left to the derivations of a process that may use `process` bytes in all, of which
/// it holds `resident` already: the block cache, which may still fill, is set aside too.
fn evaluation_memory(process: u64, resident: u64) -> u64 {
    process
        .saturating_sub(resident)
        .saturating_sub(BLOCK_CACHE_BYTES as u64)
}

/// The evaluation options of a run on `threads` CPU slots whose process may use `memory` bytes
/// in all, half the physical memory when not given. The estimates of the derivations that run at
/// once get what the process does not hold yet, and the resident memory of the process is kept
/// within the limit too. Without a known amount of memory there is no memory limit.
pub(crate) fn options(threads: usize, memory: Option<u64>, plain: bool) -> Options {
    let now = memory_now();
    let process = match (memory, now) {
        (Some(process), _) => Some(process),
        (None, Some((total, _))) if total > 0 => Some(total / 2),
        (None, _) => None,
    };
    let held = now.map_or(0, |(_, resident)| resident);
    Options {
        threads,
        memory: process.map_or(u64::MAX, |process| evaluation_memory(process, held)),
        resident: process.map(|limit| Resident {
            limit,
            probe: resident,
        }),
        plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_counts_take_binary_suffixes() {
        assert_eq!(parse_bytes("1048576"), Ok(1 << 20));
        assert_eq!(parse_bytes("512M"), Ok(512 << 20));
        assert_eq!(parse_bytes("8g"), Ok(8 << 30));
        assert_eq!(parse_bytes("2T"), Ok(2 << 40));
        for bad in ["", "M", "1.5G", "-1", "8X", "99999999999T"] {
            assert!(parse_bytes(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_evaluation_gets_what_the_process_does_not_hold() {
        let cache = BLOCK_CACHE_BYTES as u64;
        assert_eq!(evaluation_memory(4 << 30, 1 << 30), (3 << 30) - cache);
        assert_eq!(evaluation_memory(1 << 20, 1 << 30), 0);
    }

    #[test]
    fn plain_and_threads_pass_through() {
        let options = options(3, Some(u64::MAX), true);
        assert_eq!((options.threads, options.plain), (3, true));
        assert!(options.memory > 1 << 40);
        assert_eq!(options.resident.map(|r| r.limit), Some(u64::MAX));
    }

    #[test]
    fn the_resident_memory_of_this_process_is_read() {
        if memory_now().is_some() {
            assert!(resident() > 0);
        }
    }
}
