# rust-ios-device

[English](README.md) | 简体中文

一组用于操作真实 iOS 设备的 Rust 库、语言绑定和 `ios` 命令行工具。项目通过
usbmuxd、lockdown、CoreDevice 隧道、Remote Service Discovery (RSD)、
RemoteXPC 以及常见 Apple 设备服务与设备通信。

本项目目前 **仍是实验性项目，但能力面已经较宽**：适合设备自动化、协议研究、开发者工具、诊断排障，以及与
`go-ios`、`pymobiledevice3` 常见工作流做兼容性对照。稳定版本前 API 和 CLI 仍可能变化，很多服务是否可用也取决于设备型号、iOS 版本、信任状态、开发者模式、监督状态和主机上的 Apple 组件。

## 工作区内容

| 入口 | 用途 |
| --- | --- |
| `ios-core` | Rust 库，包含发现、配对、lockdown、usbmux、隧道、XPC/RSD、协议编解码和 feature-gated 服务客户端。 |
| `ios-cli` | 面向终端用户的 CLI，二进制名为 `ios`，启用完整 `ios-core` 服务能力。 |
| `ios-py` | PyO3 模块，包名为 `rust-ios-device-tunnel`，导入名为 `ios_rs`，重点覆盖设备列表和 CoreDevice 隧道工作流。 |
| `ios-ffi` | C ABI 包装，构建静态/动态库以及 `ios_rs.h` 头文件。 |
| `docs/` | 构建、用法、架构、feature flags、CLI 对照、隧道、协议、Python 绑定和故障排查文档。 |

## 能力概览

`rust-ios-device` 目前覆盖这些主要能力：

- 通过 usbmuxd 与 Bonjour/mDNS 发现设备，并监听连接/断开事件。
- Lockdown 访问、TLS 会话、配对记录、SRP 配对、服务启动和部分设备设置。
- 经典 lockdown/usbmux 服务：AFC、House Arrest、崩溃报告、diagnostics relay、file relay、heartbeat、安装/应用管理、notification proxy、配置描述文件、provisioning profiles、截图、SpringBoard、syslog、备份辅助能力以及相关管理服务。
- iOS 17+ CoreDevice 工作流：CDTunnel、用户态和内核隧道模式、RSD 服务检查、RemoteXPC/HTTP2 传输、appservice、fileservice、diagnosticsservice、deviceinfo、Instruments、TestManager，以及设备暴露相应服务时的端口转发。
- 开发者工作流：开发者磁盘镜像挂载、DTX/Instruments、debugserver 辅助、WebInspector、XCTest 启动、WebDriverAgent 辅助、可访问性审计、抓包、符号、os_trace、进程控制和诱导设备状态。
- 设备管理与监督设备辅助：激活状态、AMFI 开发者模式辅助、arbitration、companion devices、全局 HTTP 代理、IDAM、power assertion、preboard、prepare/监督证书辅助、restore-mode 事件辅助、erase 和 restore 入口。
- 协议基础模块：usbmuxd、lockdown、AFC、DTX、OPACK、NSKeyedArchiver、XPC、HTTP/2 XPC、TLV、TLS/PSK 和隧道包转发。
- 面向非 Rust 工具的 Python 与 C 集成接口，可复用设备发现和隧道能力。

简短地说：日常检查和自动化用 CLI；写 Rust 工具用 `ios-core`；Python 里需要用户态隧道桥接时用 `ios_rs`；C 兼容消费者用 `ios-ffi`。

## 环境要求

- Rust 1.80 或更新版本。
- 大多数真实设备操作需要一台已信任的实体 iOS 设备。
- 主机需要支持 usbmux：
  - macOS：通常 Finder/Xcode 提供的 Apple 设备支持即可。
  - Linux：安装并运行 `usbmuxd`；可能还需要 udev 权限。
  - Windows：安装 Apple Mobile Device Support，通常可通过 iTunes 或 Apple Devices 获取。
- Linux 构建可能需要 OpenSSL 开发文件，例如 `libssl-dev` 和 `pkg-config`。
- Windows 上使用 OpenSSL 的构建预期通过 vcpkg 链接 `x64-windows-static-md`。
- 只有使用 `ios-py` 时才需要 Python 3.9+ 和 `maturin`。

## 从源码构建

```sh
cargo build --workspace --exclude ios-py
cargo build --release --package ios-cli
```

从 checkout 运行 CLI：

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- --help
```

发布构建生成的二进制名为 `ios`。

大多数 CLI 命令默认输出 JSON，方便脚本使用。支持表格/文本输出的命令可通过 `--no-json` 切换。需要目标设备的命令在省略 `-u/--udid` 时会使用 `ios list` 返回的第一台设备；如需明确选择设备，请设置 `IOS_UDID` 或传入 `-u <UDID>`。

## 快速开始

```sh
ios list
ios info
ios lockdown get --key ProductVersion
ios syslog
ios screenshot --output screenshot.png
```

查看命令组：

```sh
ios file --help
ios apps --help
ios diagnostics --help
ios tunnel --help
ios instruments --help
ios prepare --help
```

## 常见 CLI 工作流

| 工作流 | 代表命令 |
| --- | --- |
| 发现与配对 | `list`, `listen`, `discover`, `pair`, `lockdown` |
| 设备信息与设置 | `info`, `diskspace`, `mobilegestalt`, `batterycheck`, `batteryregistry`, `activation`, `amfi` |
| 文件与容器 | `file`, `file --app`, `file --coredevice`, `crash`, `file-relay` |
| 应用与测试 | `apps list/install/uninstall/launch/kill`, `runtest`, `runwda`, `wda` |
| 日志与诊断 | `syslog`, `diagnostics`, `os-trace`, `notify`, `pcap` |
| 开发者服务 | `ddi`, `instruments`, `debugserver`, `debug`, `symbols`, `accessibility-audit`, `webinspector`, `devicestate`, `memlimitoff` |
| iOS 17+ 传输 | `tunnel start`, `tunnel serve`, `tunnel list`, `rsd services`, `rsd check`, `forward` |
| 管理与监督 | `profiles`, `provisioning`, `prepare`, `httpproxy`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` |

按任务组织的示例见 [docs/usage.md](docs/usage.md)。与 go-ios /
pymobiledevice3 的命令族对照见 [docs/cli-map.md](docs/cli-map.md)。

## CoreDevice、RSD 与 fileservice 说明

iOS 17+ 能力取决于设备实际暴露的服务面。设备可能 USB、lockdown、tunnel、RSD、AFC 和 InstallationProxy 都正常，但仍然不暴露某个具体 CoreDevice 服务。

例如 CoreDevice fileservice 使用：

- `com.apple.coredevice.fileservice.control`
- `com.apple.coredevice.fileservice.data`

排查前先检查服务是否存在：

```sh
ios rsd services --all
ios rsd check com.apple.coredevice.fileservice.control
ios file --coredevice --domain temporary ls /
```

如果 RSD 不暴露 fileservice control/data，CLI 应报告清晰的缺失服务错误。这与当前参考工具行为一致，而不是回退到另一个服务名。

## 隧道

启动单个 CoreDevice 隧道：

```sh
ios tunnel start --userspace
```

运行给集成工具使用的本地隧道管理器：

```sh
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

建议优先使用用户态模式。它暴露一个本地 TCP 代理：客户端代理流量前需要先发送 16 字节 IPv6 地址，再发送 4 字节小端序端口号。内核 TUN 模式也可用，但可能需要管理员/root 权限。

详情见 [docs/tunnel.md](docs/tunnel.md)。

## Rust API

`ios-core` 默认不启用具体服务 feature。请只启用工具实际需要的服务：

```toml
[dependencies]
ios-core = { version = "0.1.5", features = ["afc", "syslog"] }
```

需要更宽能力面时可使用分组 feature：`classic`、`developer`、`management`、`ios17` 或 `full`。CLI 使用 `full`；库用户通常不建议这么宽。

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

Feature 细节见 [docs/features.md](docs/features.md)，架构说明见 [docs/architecture.md](docs/architecture.md)。

## Python 绑定

安装 Python 包：

```sh
pip install rust-ios-device-tunnel
```

从当前 checkout 构建本地模块：

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

`crates/ios-py/examples/pymobiledevice3_coredevice_bridge.py` 展示了如何让
pymobiledevice3 RemoteXPC 代码跑在 Rust 用户态隧道之上。

## C FFI

构建 C 兼容库和头文件：

```sh
cargo build --release -p ios-ffi
```

FFI crate 为不能直接调用 Rust API 的消费者暴露设备列表、配对/服务访问和隧道生命周期函数。

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

不同示例的参数可能不同；如果示例需要路径或 app identifier，请阅读源码或命令输出。

## 安全和限制

- 这不是 Apple 支持的 SDK，也不能替代 Xcode、Finder、Apple Configurator 或官方 MDM 工具。
- 并非每个命令都在所有 iOS 版本和主机系统上验证过。
- 部分服务需要开发者模式、已挂载的开发者磁盘镜像、监督模式、已安装测试 bundle 或应用特定 entitlement。
- `erase`、`restore`、`prepare`、`httpproxy`、`location`、`preboard`、描述文件管理和 backup restore 路径可能修改设备状态。请优先使用测试设备，并先阅读 `--help`。
- 配对记录和监督证书属于敏感凭据。不要提交到仓库，也不要写入日志。

## 故障排查

- 设备不可见：解锁设备、信任主机、重新连接 USB，并确认 usbmuxd 或 Apple Mobile Device Support 正常。
- 配对失败：只有在理解影响时才删除旧配对记录，然后从已解锁设备重新配对。
- 隧道失败：确认设备暴露所需 CoreDevice tunnel/RSD 服务；适合时回退到经典 lockdown/usbmux 服务。
- CoreDevice fileservice 失败：先检查 RSD 是否包含 control/data 服务名，再判断是否是实现问题。
- 内核隧道失败：改用用户态模式，或使用创建 TUN 接口所需的权限运行。
- 开发者服务失败：按需启用开发者模式，并在服务需要时挂载兼容的开发者磁盘镜像。

更多细节见 [docs/troubleshooting.md](docs/troubleshooting.md)。

## 贡献

欢迎贡献。开发环境、测试、格式化、lint 和 PR 要求见 [CONTRIBUTING.md](CONTRIBUTING.md)。

常用检查：

```sh
cargo build --workspace --exclude ios-py
cargo test --workspace --exclude ios-core --exclude ios-py
cargo test -p ios-core --all-features
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 许可证

可任选以下许可证之一使用：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

## 致谢

本项目受到更广泛的 iOS 设备工具生态启发，尤其是：

- [go-ios](https://github.com/danielpaulus/go-ios.git)
- [pymobiledevice3](https://github.com/doronz88/pymobiledevice3.git)

兼容性仅在本仓库代码和测试支持的范围内实现。
