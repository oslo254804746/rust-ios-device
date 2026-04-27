/// AFC debug: test list_dir WITHOUT calling device_info first
#[tokio::main]
async fn main() {
    let udid = std::env::args().nth(1).expect("Usage: afc_debug <UDID>");
    println!("=== AFC Debug for {} ===", udid);

    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts)
        .await
        .expect("connect failed");
    println!("[+] Device connected");

    let stream = device
        .connect_service("com.apple.afc")
        .await
        .expect("afc connect failed");
    println!("[+] AFC service connected");

    // Test 1: list_dir WITHOUT device_info first (simulates CLI behavior)
    println!("[test 1] list_dir / (packet_num=1, no warm-up)...");
    let mut afc = ios_core::afc::AfcClient::new(stream);
    match afc.list_dir("/").await {
        Ok(entries) => println!("[+] OK: {:?}", entries),
        Err(e) => println!("[-] FAIL: {e}"),
    }

    // Test 2: try a second request
    println!("[test 2] list_dir / (packet_num=2)...");
    match afc.list_dir("/").await {
        Ok(entries) => println!("[+] OK: {:?}", entries),
        Err(e) => println!("[-] FAIL: {e}"),
    }

    // Reconnect and test device_info first
    let stream2 = device
        .connect_service("com.apple.afc")
        .await
        .expect("afc2 connect failed");
    let mut afc2 = ios_core::afc::AfcClient::new(stream2);
    println!("[test 3] device_info first (packet_num=1)...");
    match afc2.device_info().await {
        Ok(info) => println!("[+] device_info OK: {:?}", info),
        Err(e) => println!("[-] device_info FAIL: {e}"),
    }
    println!("[test 4] list_dir after device_info (packet_num=2)...");
    match afc2.list_dir("/").await {
        Ok(entries) => println!(
            "[+] OK ({} entries): {:?}",
            entries.len(),
            &entries[..entries.len().min(3)]
        ),
        Err(e) => println!("[-] FAIL: {e}"),
    }
}
