//! Read-only process observations. Handles and TCP endpoints are separate
//! counters; neither is relabelled as Unix file descriptors or socket handles.
use super::*;
use std::mem::size_of;
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, FILETIME, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, GetProcessTimes,
};

/// Diagnostic only: enumerate the default heap used by Rust's Windows system
/// allocator, never third-party heaps whose lifetime we do not own. No Rust
/// allocation, logging or await occurs while this heap is locked.
pub(super) fn default_heap_sample() -> serde_json::Value {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NO_MORE_ITEMS};
    use windows_sys::Win32::System::Memory::{
        GetProcessHeap, HeapLock, HeapUnlock, HeapWalk, PROCESS_HEAP_ENTRY,
    };
    use windows_sys::Win32::System::SystemServices::{
        PROCESS_HEAP_ENTRY_BUSY, PROCESS_HEAP_REGION, PROCESS_HEAP_UNCOMMITTED_RANGE,
    };
    let started = MeasureInstant::now();
    let mut entry = PROCESS_HEAP_ENTRY::default();
    let mut busy_bytes = 0u64;
    let mut free_bytes = 0u64;
    let mut region_commit_bytes = 0u64;
    let mut entries = 0usize;
    let mut terminal = 0u32;
    let mut bounded = false;
    // SAFETY: GetProcessHeap's default heap lives for this entire process.
    // The initialized C entry is left unchanged between HeapWalk calls.
    let (locked, unlocked) = unsafe {
        let heap = GetProcessHeap();
        let locked = HeapLock(heap) != 0;
        if locked {
            loop {
                if HeapWalk(heap, &mut entry) == 0 {
                    terminal = GetLastError();
                    break;
                }
                entries += 1;
                if entries > 1_000_000
                    || (entries % 128 == 0 && started.elapsed() > Duration::from_secs(2))
                {
                    bounded = true;
                    break;
                }
                let flags = u32::from(entry.wFlags);
                if flags & PROCESS_HEAP_ENTRY_BUSY != 0 {
                    busy_bytes = busy_bytes
                        .saturating_add(u64::from(entry.cbData) + u64::from(entry.cbOverhead));
                } else if flags & PROCESS_HEAP_REGION != 0 {
                    region_commit_bytes = region_commit_bytes
                        .saturating_add(u64::from(entry.Anonymous.Region.dwCommittedSize));
                } else if flags & PROCESS_HEAP_UNCOMMITTED_RANGE == 0 {
                    free_bytes = free_bytes
                        .saturating_add(u64::from(entry.cbData) + u64::from(entry.cbOverhead));
                }
            }
            (true, HeapUnlock(heap) != 0)
        } else {
            terminal = GetLastError();
            (false, false)
        }
    };
    json!({
        "method": "default-process-heap-HeapLock-HeapWalk-HeapUnlock",
        "complete": locked && unlocked && !bounded && terminal == ERROR_NO_MORE_ITEMS,
        "locked": locked, "unlocked": unlocked, "terminal_win32_code": terminal, "bound_reached": bounded,
        "entries": entries, "busy_bytes": busy_bytes, "free_block_bytes": free_bytes,
        "region_commit_bytes": region_commit_bytes, "elapsed_ns": started.elapsed().as_nanos(),
        "scope": "default heap only; excludes other heaps, stacks, mappings; diagnostic not a throughput sample",
    })
}

fn own_tcp_endpoints(family: u32, row_size: usize) -> Option<usize> {
    let mut size = 0u32;
    // SAFETY: first call only queries the required size; no table is dereferenced.
    if unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            family,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    } != ERROR_INSUFFICIENT_BUFFER
    {
        return None;
    }
    // The table can change between queries. Retry a bounded three times and
    // refuse unreasonable system-wide tables before allocation.
    for _ in 0..3 {
        if !(4..=16 * 1024 * 1024).contains(&size) {
            return None;
        }
        let mut bytes = vec![0u8; size as usize];
        // SAFETY: capacity equals the requested byte count. Windows reports a
        // new required count on insufficient-buffer; that result is not parsed.
        let result = unsafe {
            GetExtendedTcpTable(
                bytes.as_mut_ptr().cast(),
                &mut size,
                0,
                family,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        if result == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if result != NO_ERROR || size as usize > bytes.len() || size < 4 {
            return None;
        }
        let entries = u32::from_ne_bytes(bytes[..4].try_into().ok()?) as usize;
        let end = entries.checked_mul(row_size)?.checked_add(4)?;
        if end > size as usize {
            return None;
        }
        let mut own = 0;
        for row in bytes[4..end].chunks_exact(row_size) {
            // SAFETY: each checked slice is the exact C row size, and read_unaligned
            // avoids relying on Vec<u8> alignment. Fields contain integers only.
            let pid = unsafe {
                if family == u32::from(AF_INET) {
                    std::ptr::read_unaligned(row.as_ptr().cast::<MIB_TCPROW_OWNER_PID>())
                        .dwOwningPid
                } else {
                    std::ptr::read_unaligned(row.as_ptr().cast::<MIB_TCP6ROW_OWNER_PID>())
                        .dwOwningPid
                }
            };
            own += usize::from(pid == std::process::id());
        }
        return Some(own);
    }
    None
}

pub(super) fn sample() -> serde_json::Value {
    let mut memory = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let mut extended = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let mut handles = 0;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the pseudo-handle names the current process and needs no CloseHandle.
    // All output pointers refer to appropriately sized initialized C structures.
    let (memory_ok, extended_ok, handles_ok, times_ok) = unsafe {
        let process = GetCurrentProcess();
        // EX2 is available on recent Windows. A native rejection falls back to
        // EX; older systems retain their useful counters with explicit nulls.
        let extended_ok = GetProcessMemoryInfo(
            process,
            (&mut extended as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            extended.cb,
        ) != 0;
        if extended_ok {
            memory.WorkingSetSize = extended.WorkingSetSize;
            memory.PeakWorkingSetSize = extended.PeakWorkingSetSize;
            memory.PrivateUsage = extended.PrivateUsage;
        }
        (
            extended_ok
                || GetProcessMemoryInfo(
                    process,
                    (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                    memory.cb,
                ) != 0,
            extended_ok,
            GetProcessHandleCount(process, &mut handles) != 0,
            GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) != 0,
        )
    };
    let time_ns = |time: FILETIME| {
        (u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime)).checked_mul(100)
    };
    let tcp_v4 = own_tcp_endpoints(u32::from(AF_INET), size_of::<MIB_TCPROW_OWNER_PID>());
    let tcp_v6 = own_tcp_endpoints(u32::from(AF_INET6), size_of::<MIB_TCP6ROW_OWNER_PID>());
    json!({
        "method": "windows-GetProcessMemoryInfo-GetProcessHandleCount-GetProcessTimes-GetExtendedTcpTable",
        "rss_bytes": memory_ok.then_some(memory.WorkingSetSize),
        "peak_rss_bytes": memory_ok.then_some(memory.PeakWorkingSetSize),
        "private_commit_bytes": memory_ok.then_some(memory.PrivateUsage),
        "private_working_set_bytes": extended_ok.then_some(extended.PrivateWorkingSetSize),
        "non_private_working_set_bytes": extended_ok.then(|| extended.WorkingSetSize.checked_sub(extended.PrivateWorkingSetSize)).flatten(),
        "process_handle_count": handles_ok.then_some(handles),
        "cpu_kernel_ns": times_ok.then(|| time_ns(kernel)).flatten(),
        "cpu_user_ns": times_ok.then(|| time_ns(user)).flatten(),
        "tcp_endpoint_count": tcp_v4.zip(tcp_v6).map(|(v4, v6)| v4 + v6),
        "tcp_v4_endpoint_count": tcp_v4, "tcp_v6_endpoint_count": tcp_v6,
        "fd_count": null, "socket_fd_count": null, "process_threads": null,
        "counter_scope": "own process working set/private commit; all own handles; IPv4/IPv6 TCP endpoint rows including listeners/TIME_WAIT; no handle type attribution",
    })
}

#[tokio::test]
async fn actual_socket_and_file_handles_are_observed_and_released() {
    let before = sample();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let path = std::env::temp_dir().join(format!("ios-l04-handle-{}", uuid::Uuid::new_v4()));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap();
    let active = sample();
    assert!(active["rss_bytes"].as_u64().unwrap() > 0);
    assert!(active["private_commit_bytes"].as_u64().unwrap() > 0);
    assert!(
        active["process_handle_count"].as_u64().unwrap()
            >= before["process_handle_count"].as_u64().unwrap() + 2
    );
    assert!(
        active["tcp_v4_endpoint_count"].as_u64().unwrap()
            > before["tcp_v4_endpoint_count"].as_u64().unwrap()
    );
    drop(listener);
    drop(file);
    std::fs::remove_file(path).unwrap();
    let released = sample();
    assert!(
        released["process_handle_count"].as_u64().unwrap()
            < active["process_handle_count"].as_u64().unwrap()
    );
    assert!(
        released["tcp_v4_endpoint_count"].as_u64().unwrap()
            < active["tcp_v4_endpoint_count"].as_u64().unwrap()
    );
}

#[test]
fn default_heap_walk_observes_a_live_rust_allocation_and_its_release() {
    const ALLOCATION: u64 = 1024 * 1024;
    let before = default_heap_sample();
    let payload = vec![0xa5u8; ALLOCATION as usize];
    std::hint::black_box(&payload);
    let live = default_heap_sample();
    drop(payload);
    let released = default_heap_sample();
    for sample in [&before, &live, &released] {
        assert_eq!(sample["complete"], true, "{sample}");
    }
    let before_busy = before["busy_bytes"].as_u64().unwrap();
    let live_busy = live["busy_bytes"].as_u64().unwrap();
    let released_busy = released["busy_bytes"].as_u64().unwrap();
    assert!(live_busy >= before_busy + ALLOCATION);
    // JSON bookkeeping may allocate too; require the large original block to
    // disappear without equating committed heap capacity with live objects.
    assert!(released_busy + ALLOCATION / 2 < live_busy);
}
