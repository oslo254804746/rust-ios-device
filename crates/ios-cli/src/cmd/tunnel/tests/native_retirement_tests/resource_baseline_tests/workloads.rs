//! Host device peers, exercising the actual native stack and service parsers.
use super::*;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Poll the same pinned operation throughout sampling. In particular, a timer
/// never drops/recreates a partly read SyslogStream message or native frame.
pub(super) async fn sampled_workload(
    operation: impl std::future::Future<Output = Result<serde_json::Value>>,
    state: &TunnelAgentState,
) -> Result<serde_json::Value> {
    const PERIOD: Duration = Duration::from_millis(10);
    let start = MeasureInstant::now();
    let observe = || {
        let mut sample = proc_sample();
        sample["tokio_alive_tasks"] = json!(baseline());
        sample["native_occupied"] = json!(state.inner.native.snapshot().occupied());
        sample
    };
    let initial = observe();
    let mut counters = std::collections::BTreeMap::<String, (u64, u64, u64, u64, usize)>::new();
    let mut count = 0usize;
    let mut prior = start.elapsed().as_nanos();
    let mut maximum_gap = 0;
    let mut record = |sample: &serde_json::Value, now: u128| {
        count += 1;
        maximum_gap = maximum_gap.max(now.saturating_sub(prior));
        prior = now;
        if let Some(values) = sample.as_object() {
            for (name, value) in values {
                if let Some(value) = value.as_u64() {
                    let entry = counters
                        .entry(name.clone())
                        .or_insert((value, value, value, value, 0));
                    entry.0 = entry.0.min(value);
                    entry.1 = entry.1.max(value);
                    entry.3 = value;
                    entry.4 += 1;
                }
            }
        }
    };
    record(&initial, start.elapsed().as_nanos());
    let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + PERIOD, PERIOD);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(operation);
    let result = loop {
        tokio::select! {
            result = &mut operation => break result,
            _ = timer.tick() => record(&observe(), start.elapsed().as_nanos()),
        }
    };
    let final_sample = observe();
    record(&final_sample, start.elapsed().as_nanos());
    let counters: serde_json::Map<_, _> = counters.into_iter().map(|(name, (minimum, maximum, first, last, samples))| {
        (name, json!({"minimum": minimum, "maximum": maximum, "first": first, "last": last, "samples": samples}))
    }).collect();
    let mut metrics = result?;
    metrics["resources"] = json!({
        "method": "one-owned-pinned-workload-future-concurrent-monotonic-sampler-no-detached-task",
        "period_ns": PERIOD.as_nanos(), "maximum_observed_gap_ns": maximum_gap,
        "elapsed_ns": start.elapsed().as_nanos(), "samples": count,
        "process_method": initial["method"], "counter_scope": initial["counter_scope"], "counters": counters,
        "peak_scope": "sampled process counters; OS peak_rss_bytes is lifetime high water; fixture and manager share the process",
    });
    Ok(metrics)
}

fn checksum(parts: &[&[u8]]) -> u16 {
    let mut sum = 0u32;
    for part in parts {
        for chunk in part.chunks(2) {
            sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
        }
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn echo_packet(sequence: u16, payload: &[u8]) -> Vec<u8> {
    let source = "fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets();
    let target = "fd00::2".parse::<std::net::Ipv6Addr>().unwrap().octets();
    let mut icmp = vec![128, 0, 0, 0, 0x04, 0x04];
    icmp.extend_from_slice(&sequence.to_be_bytes());
    icmp.extend_from_slice(payload);
    let length = (icmp.len() as u32).to_be_bytes();
    let next = [0, 0, 0, 58];
    let value = checksum(&[&source, &target, &length, &next, &icmp]);
    icmp[2..4].copy_from_slice(&value.to_be_bytes());
    let mut packet = vec![0x60, 0, 0, 0];
    packet.extend_from_slice(&(icmp.len() as u16).to_be_bytes());
    packet.extend_from_slice(&[58, 64]);
    packet.extend_from_slice(&source);
    packet.extend_from_slice(&target);
    packet.extend_from_slice(&icmp);
    packet
}

pub(super) async fn measure_throughput_workload(
    peer: &mut TcpStream,
    payload_bytes: usize,
) -> Result<serde_json::Value> {
    let started = MeasureInstant::now();
    let mut sent = 0usize;
    let mut sequence = 0u16;
    let mut latencies = Vec::new();
    let mut expected_hash = Sha256::new();
    let mut received_hash = Sha256::new();
    // Each reply traverses the manager-owned native reader, smoltcp ICMPv6
    // processing and native writer. A bare localhost copy cannot satisfy it.
    while sent < payload_bytes {
        let mut pending = Vec::with_capacity(32);
        for _ in 0..32 {
            if sent == payload_bytes {
                break;
            }
            let length = (payload_bytes - sent).min(1232);
            let data: Vec<_> = (0..length)
                .map(|index| ((sent + index) % 251) as u8)
                .collect();
            let packet = echo_packet(sequence, &data);
            let before = MeasureInstant::now();
            tokio::time::timeout(CLEANUP, peer.write_all(&packet)).await??;
            expected_hash.update(&data);
            pending.push((sequence, data, before));
            sequence = sequence.wrapping_add(1);
            sent += length;
        }
        for (sequence, expected, before) in pending {
            let mut header = [0u8; 40];
            tokio::time::timeout(CLEANUP, peer.read_exact(&mut header)).await??;
            let length = u16::from_be_bytes([header[4], header[5]]) as usize;
            anyhow::ensure!(
                header[0] >> 4 == 6 && header[6] == 58 && length == expected.len() + 8,
                "native echo reply framing differs"
            );
            let mut reply = vec![0u8; length];
            tokio::time::timeout(CLEANUP, peer.read_exact(&mut reply)).await??;
            anyhow::ensure!(
                reply[0] == 129
                    && reply[1] == 0
                    && reply[4..6] == [0x04, 0x04]
                    && reply[6..8] == sequence.to_be_bytes()
                    && reply[8..] == expected,
                "native echo reply differs"
            );
            received_hash.update(&reply[8..]);
            latencies.push(before.elapsed().as_nanos());
        }
    }
    anyhow::ensure!(
        expected_hash.finalize() == received_hash.finalize(),
        "native payload hash differs"
    );
    latencies.sort_unstable();
    let elapsed = started.elapsed();
    Ok(
        json!({"status": "passed", "production_path": "manager-original-UserspaceTunDevice-reader-smoltcp-ICMPv6-writer",
        "transferred_bytes": sent, "integrity_verified": true, "elapsed_nanoseconds": elapsed.as_nanos(),
        "throughput_bytes_per_sec": sent as f64 / elapsed.as_secs_f64(),
        "p50_chunk_ns": latencies[(latencies.len() - 1) / 2], "p95_chunk_ns": latencies[(latencies.len() * 95).div_ceil(100) - 1]}),
    )
}

async fn tcp_pair() -> Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let (client, accepted) = tokio::try_join!(
        TcpStream::connect(listener.local_addr()?),
        listener.accept()
    )?;
    client.set_nodelay(true)?;
    accepted.0.set_nodelay(true)?;
    Ok((client, accepted.0))
}

async fn afc_reply(
    stream: &mut TcpStream,
    packet: u64,
    opcode: u64,
    header: &[u8],
    data: &[u8],
) -> Result<()> {
    let mut wire = Vec::with_capacity(40 + header.len() + data.len());
    for field in [
        0x4141504c36414643u64,
        (40 + header.len() + data.len()) as u64,
        (40 + header.len()) as u64,
        packet,
        opcode,
    ] {
        wire.extend_from_slice(&field.to_le_bytes());
    }
    wire.extend_from_slice(header);
    wire.extend_from_slice(data);
    stream.write_all(&wire).await?;
    Ok(())
}

async fn serve_afc(mut stream: TcpStream, root: &std::path::Path, count: usize) -> Result<usize> {
    let mut handle = None;
    let mut requests = 0;
    loop {
        let mut first = [0u8; 1];
        if stream.read(&mut first).await? == 0 {
            break;
        }
        let mut header = [0u8; 40];
        header[0] = first[0];
        stream.read_exact(&mut header[1..]).await?;
        let field =
            |index: usize| u64::from_le_bytes(header[index * 8..index * 8 + 8].try_into().unwrap());
        let entire = field(1) as usize;
        let this = field(2) as usize;
        anyhow::ensure!(
            field(0) == 0x4141504c36414643u64
                && 40 <= this
                && this <= entire
                && entire <= 1024 * 1024 + 64,
            "AFC fixture request bound"
        );
        let packet = field(3);
        let opcode = field(4);
        let mut bytes = vec![0u8; entire - 40];
        stream.read_exact(&mut bytes).await?;
        let split = this - 40;
        let (hp, data) = bytes.split_at(split);
        match opcode {
            13 => {
                anyhow::ensure!(handle.is_none() && hp.len() >= 10, "AFC open shape");
                let mode = u64::from_le_bytes(hp[..8].try_into().unwrap());
                let path = std::str::from_utf8(&hp[8..hp.len() - 1])?;
                anyhow::ensure!(
                    hp.last() == Some(&0)
                        && (0..count).any(|index| path == format!("/l04-{index}")),
                    "AFC fixture path differs"
                );
                let local = root.join(&path[1..]);
                let file = if mode == ios_core::afc::AfcClient::<TcpStream>::FILE_MODE_READ_ONLY {
                    tokio::fs::File::open(local).await?
                } else {
                    anyhow::ensure!(mode == ios_core::afc::AfcClient::<TcpStream>::FILE_MODE_WRITE_ONLY_CREATE_TRUNC, "AFC open mode differs");
                    tokio::fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(local)
                        .await?
                };
                handle = Some(file);
                afc_reply(&mut stream, packet, 14, &1u64.to_le_bytes(), &[]).await?;
            }
            16 => {
                anyhow::ensure!(hp == 1u64.to_le_bytes(), "AFC write handle differs");
                handle
                    .as_mut()
                    .context("AFC write without open")?
                    .write_all(data)
                    .await?;
                afc_reply(&mut stream, packet, 1, &0u64.to_le_bytes(), &[]).await?;
            }
            15 => {
                anyhow::ensure!(
                    hp.len() == 16 && hp[..8] == 1u64.to_le_bytes(),
                    "AFC read shape"
                );
                let amount = u64::from_le_bytes(hp[8..].try_into().unwrap()) as usize;
                anyhow::ensure!(amount <= 65536, "AFC fixture read bound");
                let mut bytes = vec![0; amount];
                let read = handle
                    .as_mut()
                    .context("AFC read without open")?
                    .read(&mut bytes)
                    .await?;
                afc_reply(&mut stream, packet, 2, &[], &bytes[..read]).await?;
            }
            20 => {
                anyhow::ensure!(
                    hp == 1u64.to_le_bytes() && handle.is_some(),
                    "AFC close shape"
                );
                drop(handle.take());
                afc_reply(&mut stream, packet, 1, &0u64.to_le_bytes(), &[]).await?;
            }
            8 => {
                let path = std::str::from_utf8(&hp[..hp.len() - 1])?;
                anyhow::ensure!(
                    hp.last() == Some(&0)
                        && (0..count).any(|index| path == format!("/l04-{index}")),
                    "AFC cleanup path differs"
                );
                tokio::fs::remove_file(root.join(&path[1..])).await?;
                afc_reply(&mut stream, packet, 1, &0u64.to_le_bytes(), &[]).await?;
            }
            _ => anyhow::bail!("unexpected AFC fixture operation"),
        }
        requests += 1;
    }
    anyhow::ensure!(handle.is_none(), "AFC fixture retained an open file");
    Ok(requests)
}

pub(super) async fn measure_file_transfer_workload(
    count: usize,
    size: usize,
) -> Result<serde_json::Value> {
    let root = std::env::temp_dir().join(format!("ios-l04-afc-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir(&root).await?;
    let (client, peer) = tcp_pair().await?;
    let started = MeasureInstant::now();
    let operation = async {
        let service = async {
            let mut afc = ios_core::afc::AfcClient::new(client);
            afc.set_request_timeout(Some(CLEANUP));
            let mut samples = Vec::new();
            for index in 0..count {
                let bytes: Vec<_> = (0..size)
                    .map(|offset| ((offset + index) % 251) as u8)
                    .collect();
                let before = MeasureInstant::now();
                let path = format!("/l04-{index}");
                afc.write_file(&path, &bytes).await?;
                let disk = tokio::fs::read(root.join(&path[1..])).await?;
                anyhow::ensure!(
                    Sha256::digest(&disk) == Sha256::digest(&bytes),
                    "AFC uploaded disk hash differs"
                );
                let received = afc.read_file_with_limit(&path, size).await?;
                anyhow::ensure!(
                    received.len() == size && Sha256::digest(&received) == Sha256::digest(&bytes),
                    "AFC downloaded hash differs"
                );
                afc.remove(&path).await?;
                samples.push(before.elapsed().as_nanos());
            }
            drop(afc); // Peer must observe a real EOF before successful receipt.
            Ok::<_, anyhow::Error>(samples)
        };
        tokio::try_join!(service, serve_afc(peer, &root, count))
    };
    let result = tokio::time::timeout(Duration::from_secs(30), operation).await;
    // Only exact fixture files are removed on failure; unknown residue is kept.
    for index in 0..count {
        let path = root.join(format!("l04-{index}"));
        if tokio::fs::symlink_metadata(&path)
            .await
            .is_ok_and(|metadata| metadata.is_file() && !metadata.is_symlink())
        {
            tokio::fs::remove_file(path).await?;
        }
    }
    tokio::fs::remove_dir(&root).await?;
    let (mut samples, requests) = result??;
    samples.sort_unstable();
    let elapsed = started.elapsed();
    Ok(
        json!({"status": "passed", "production_path": "AfcClient-write_file-read_file_with_limit-remove",
        "fixture_storage": "exclusive-real-disk-files", "files_transferred": count, "total_bytes": count * size * 2,
        "elapsed_nanoseconds": elapsed.as_nanos(), "throughput_bytes_per_sec": (count * size * 2) as f64 / elapsed.as_secs_f64(),
        "integrity_verified": true, "strict_peer_eof": true, "owned_files_removed": true, "protocol_requests": requests,
        "p50_file_ns": samples[(samples.len() - 1) / 2], "p95_file_ns": samples[(samples.len() * 95).div_ceil(100) - 1]}),
    )
}

pub(super) async fn measure_service_stream_workload(
    duration: Duration,
) -> Result<serde_json::Value> {
    let (client, mut peer) = tcp_pair().await?;
    let start = MeasureInstant::now();
    let producer = async {
        let mut tick = tokio::time::interval(Duration::from_millis(1));
        let mut sequence = 0;
        while start.elapsed() < duration {
            tick.tick().await;
            let message = format!("l04-syslog-{sequence:010}\\M-C\\M-)\0");
            peer.write_all(message.as_bytes()).await?;
            sequence += 1;
        }
        peer.shutdown().await?;
        let mut byte = [0];
        anyhow::ensure!(
            peer.read(&mut byte).await? == 0,
            "syslog client did not close with EOF"
        );
        Ok::<_, anyhow::Error>(sequence)
    };
    let consumer = async {
        let mut stream = ios_core::syslog::SyslogStream::new(client);
        let mut events = 0;
        loop {
            match stream.next_message().await {
                Ok(message) => {
                    anyhow::ensure!(
                        message == format!("l04-syslog-{events:010}é"),
                        "production syslog decoding/sequence differs"
                    );
                    events += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error.into()),
            }
        }
        drop(stream);
        Ok::<_, anyhow::Error>(events)
    };
    let (sent, received) = tokio::time::timeout(duration + CLEANUP, async {
        tokio::try_join!(producer, consumer)
    })
    .await??;
    let elapsed = start.elapsed();
    anyhow::ensure!(
        elapsed >= duration && sent == received && received > 0,
        "syslog duration/event receipt incomplete"
    );
    Ok(
        json!({"status": "passed", "production_path": "SyslogStream-next_message-NUL-framing-BSD-vis-decode",
        "requested_duration_ns": duration.as_nanos(), "stream_duration_ns": elapsed.as_nanos(), "events_streamed": received,
        "events_per_second": received as f64 / elapsed.as_secs_f64(), "dropped_events": 0,
        "strict_peer_eof": true}),
    )
}
