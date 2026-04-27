# rust-ios-device

[English](README.md) | 简体中文

用于通过 usbmuxd、lockdown、CoreDevice/RemoteXPC 以及常见设备服务与 iOS 设备通信的 Rust 库和命令行工具。

本项目目前仍处于 **实验性** 阶段。它适用于开发、测试和协议研究，但 API 和 CLI 在稳定版本发布前可能发生变化。部分服务需要真实且已信任的设备，并且行为可能因 iOS 版本、主机操作系统、配对状态和已安装的 Apple 组件而异。

## 功能特性

- 通过 usbmuxd 发现 USB 设备并监听设备事件。
- 支持 Lockdown 客户端、TLS 会话、配对记录和配对辅助功能。
- 支持跨多个 iOS 版本使用的 lockdown/usbmux 服务路径。
- 支持在具备 CoreDevice 隧道路由的 iOS 版本上通过 CoreDeviceProxy/CDTunnel 建立隧道，并提供用户态和内核 TUN 模式。
- 支持 Remote Service Discovery (RSD)、HTTP/2 XPC 传输、OPACK、NSKeyedArchiver、AFC、DTX、lockdown、usbmuxd 和 XPC 协议编解码。
- CLI 命令覆盖设备信息、配对、文件操作、应用管理、syslog、截图、诊断、预置/配置描述文件、崩溃报告、Instruments、WebInspector、debugserver、备份/恢复辅助功能和隧道管理。
- 基于 feature gate 的服务客户端，覆盖 AFC、应用、syslog、截图、DTX/Instruments、TestManager、可访问性审计、开发者磁盘镜像挂载、pcap、WebInspector 及相关服务。
- Python 绑定（`rust-ios-device-tunnel`，导入名为 `ios_rs`），用于设备列表和用户态隧道工作流。
- C FFI 绑定，用于设备列表、lockdown 查询和隧道元数据。

## 非目标和限制

- 这不是 Apple 支持的 SDK，也不能替代 Xcode、Finder、Apple Configurator 或官方 MDM 工具。
- 并非每个命令都在所有 iOS 版本上验证过。部分高级命令更适合作为协议实验使用。
- CoreDevice 和隧道路由需要已信任设备、兼容的 iOS 版本以及正确的配对材料。
- 内核 TUN 模式可能需要管理员/root 权限。用户态模式通常更容易运行。
- 部分服务需要开发者模式、已挂载的开发者磁盘镜像、已安装的测试 bundle、监督模式或应用特定 entitlement。
- 会修改设备状态的命令可能具有破坏性。使用 profile、erase、restore、backup restore、location、preboard 和监督相关命令前，请先阅读命令帮助。

## 仓库结构

| Crate | 用途 |
| --- | --- |
| `ios-core` | 公开 Rust 库。包含协议编解码、usbmuxd、lockdown、隧道、XPC/RSD、feature-gated 服务客户端、发现、配对和高层设备 API。 |
| `ios-cli` | `ios` 命令行工具。 |
| `ios-py` | PyO3 Python 扩展模块。目前未发布到 crates.io。 |
| `ios-ffi` | C ABI 包装。目前未发布到 crates.io。 |

## 环境要求

- Rust 1.75 或更新版本。
- 大多数真实设备操作需要一台已信任的 iOS 设备。
- 主机需要支持 usbmux：
  - macOS：通常通过 Xcode/Finder 组件提供 Apple 设备支持。
  - Linux：安装并运行 `usbmuxd`；可能需要配置 udev 权限。
  - Windows：安装 Apple Mobile Device Support，通常可通过 iTunes 或 Apple Devices 获取。
- Linux 上的部分构建可能需要 OpenSSL 开发头文件。CI 会安装 `libssl-dev` 和 `pkg-config`。
- 仅在使用 `ios-py` 时需要 Python 3.9+ 和 `maturin`。

## 构建

```sh
cargo build --workspace --exclude ios-py
cargo build --release --package ios-cli
```

从源码运行 CLI：

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- --help
```

发布构建生成的二进制文件名为 `ios`。

## Feature flags

`ios-core` 默认不启用具体服务 feature。按需启用你的应用实际使用的服务：

```toml
[dependencies]
ios-core = { version = "0.1.1", features = ["afc", "syslog"] }
```

如果在构建覆盖面较广的工具，可以使用 `classic`、`developer`、`management`、`ios17` 或 `full` 等分组 feature。CLI 会启用 `full`；库用户通常应选择更小的 feature 集。参阅 [docs/features.md](docs/features.md)。

## 快速开始

列出可见设备：

```sh
ios list
```

读取基本设备信息：

```sh
ios -u <UDID> info
ios -u <UDID> lockdown get --key ProductVersion
```

流式读取 syslog：

```sh
ios -u <UDID> syslog
```

截取屏幕：

```sh
ios -u <UDID> screenshot --output screenshot.png
```

查看具体命令选项：

```sh
ios tunnel --help
ios file --help
ios apps --help
ios instruments --help
```

## CoreDevice 隧道

为已信任设备启动隧道：

```sh
ios -u <UDID> tunnel start --userspace
```

运行隧道管理器 HTTP 服务：

```sh
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

用户态隧道会暴露一个本地 TCP 代理。客户端在代理流量前，需要先发送 16 字节 IPv6 地址，再发送 4 字节小端序端口号。

## 库示例

```rust
use ios_core::{ConnectOptions, list_devices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = list_devices().await?;
    let Some(device) = devices.first() else {
        println!("no device found");
        return Ok(());
    };

    let connected = ios_core::connect(&device.udid, ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    }).await?;

    let version = connected.product_version().await?;
    println!("{} runs iOS {}", connected.info.udid, version);
    Ok(())
}
```

如需更底层访问，请使用 `ios-core` 暴露的模块，例如 `ios_core::mux`、
`ios_core::lockdown`、`ios_core::xpc`，以及启用相应 feature 后在 crate 根部重导出的
`ios_core::afc`、`ios_core::apps`、`ios_core::syslog` 等服务模块。

## Python 绑定

安装已发布的包：

```sh
uv pip install rust-ios-device-tunnel
```

从 checkout 构建并安装本地 Python 模块：

```sh
cd crates/ios-py
uvx maturin develop
```

示例：

```python
import ios_rs

devices = ios_rs.list_devices()
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
print(tunnel.connect_info())

with tunnel.asyncio_proxy():
    # asyncio.open_connection() calls to the device tunnel address are routed
    # through the userspace proxy while this context is active.
    pass

tunnel.close()
```

## 示例

CLI crate 包含 Rust 示例：

```sh
cargo run -p ios-cli --example device_info -- <UDID>
cargo run -p ios-cli --example app_list -- <UDID>
cargo run -p ios-cli --example file_transfer -- <UDID>
cargo run -p ios-cli --example screenshot -- <UDID>
cargo run -p ios-cli --example syslog_stream -- <UDID>
cargo run -p ios-cli --example instruments_cpu -- <UDID>
```

不同示例需要的参数可能不同；如果命令需要额外路径，请使用 `--help` 或阅读示例源码。

## 故障排查

- usbmuxd 报 `No such file or directory` 或连接被拒绝：确认 usbmuxd 或 Apple Mobile Device Support 已安装并正在运行。
- 设备未出现：解锁设备、信任主机、重新连接 USB，并检查主机权限。
- 配对失败：只有在理解影响的情况下才删除过期配对记录，然后从已解锁设备重新配对。
- 隧道在部分设备上失败：确认设备/iOS 版本暴露 CoreDevice 隧道服务；较旧服务路径请使用 lockdown/usbmux 命令。
- 内核隧道失败：改用用户态模式，或使用创建 TUN 接口所需的权限运行。
- 开发者服务失败：在需要时启用开发者模式，并在服务依赖开发者磁盘镜像时挂载合适的镜像。

更多细节请参阅 [docs/troubleshooting.md](docs/troubleshooting.md)。

## 路线图

- 改进 macOS、Linux 和 Windows 上的真实设备验证。
- 稳定高层 Rust API，并记录服务级契约。
- 扩展常见工作流示例。
- 增强隧道、RemoteXPC 和开发者服务在不同 iOS 版本上的兼容性。
- 改进 Python 和 C 绑定的打包流程。

## 贡献

欢迎贡献。开发环境设置、测试要求和 PR 指南请参阅 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 安全

请私下报告漏洞。参阅 [SECURITY.md](SECURITY.md)。

## 许可证

可任选以下许可证之一使用：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

## 致谢

本项目受到更广泛的 iOS 设备工具生态启发。特别感谢：

- [go-ios](https://github.com/danielpaulus/go-ios.git)
- [pymobiledevice3](https://github.com/doronz88/pymobiledevice3.git)

兼容性仅在本仓库代码和测试支持的范围内实现。
