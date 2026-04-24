# ios-lockdown

Lockdown protocol implementation for pairing, sessions, TLS, and service startup.

This is a library crate in the [`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device) workspace.

## Highlights

- Lockdown request/response client over any async stream.
- Pair record, SRP, verify-pair, and supervised-pairing helpers.
- Service discovery and TLS session setup for higher-level device services.

## Install

```toml
[dependencies]
ios-lockdown = "0.1.1"
```

## Example

```rust,no_run
use ios_lockdown::LockdownClient;

# async fn run<S>(stream: S) -> anyhow::Result<()>
# where
#     S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
# {
let mut lockdown = LockdownClient::connect_with_stream(stream).await?;
let version = lockdown.product_version().await?;
println!("iOS {version}");
# Ok(())
# }
```

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-lockdown>

## License

Licensed under either of Apache-2.0 or MIT at your option.
