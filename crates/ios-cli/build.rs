fn main() {
    // Async state is heap-backed, but debug builds still construct/poll nested
    // TLS and protocol futures on the entry thread. The MSVC default 1 MiB
    // reservation overflows on real apps/XCTest paths. Reserve 8 MiB without
    // changing the initial commit or adding a detached execution thread.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bin=ios=/STACK:8388608");
        } else {
            println!("cargo:rustc-link-arg-bin=ios=-Wl,--stack,8388608");
        }
    }
}
