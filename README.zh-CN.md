# rust-ios-device

[English](README.md) | 简体中文

一组用于操作真实 iOS 设备的 Rust 库、语言绑定和 `ios` 命令行工具，通过
usbmuxd、lockdown、CoreDevice/RemoteXPC 与常见 Apple 设备服务与设备通信。

[![Crates.io — ios-core](https://img.shields.io/crates/v/ios-core.svg?label=ios-core)](https://crates.io/crates/ios-core)
[![Crates.io — ios-cli](https://img.shields.io/crates/v/ios-cli.svg?label=ios-cli)](https://crates.io/crates/ios-cli)
[![PyPI — rust-ios-device-tunnel](https://img.shields.io/pypi/v/rust-ios-device-tunnel.svg?label=rust-ios-device-tunnel)](https://pypi.org/project/rust-ios-device-tunnel/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可证)
[![MSRV](https://img.shields.io/badge/MSRV-1.80-orange.svg)](#环境要求)

> **状态：实验性。** 项目能力面已经较宽，可用于自动化、协议研究和开发者工具，
> 但稳定版本前公开 API 与 CLI 仍可能变化。具体服务是否可用取决于 iOS 版本、
> 设备信任与配对状态、Developer Mode、监督状态以及主机的 Apple Mobile Device 组件。

## 亮点

- **跨平台 CLI（`ios`）**——54+ 子命令，覆盖设备、文件、应用、Instruments、
  调试、描述文件、恢复、监督和隧道。
- **iOS 17+ 一等支持**——CoreDevice 隧道（用户态与内核 TUN）、RSD 服务发现、
  HTTP/2 RemoteXPC、appservice、fileservice、diagnosticsservice、deviceinfo、
  pasteboard、Instruments 与 TestManager。
- **Lockdown 经典服务**——AFC、House Arrest、syslog、截图、配置/provisioning
  描述文件、崩溃报告、diagnostics relay、notification proxy、SpringBoard、
  备份等。
- **开发者工作流**——开发者磁盘镜像挂载、DTX/Instruments、debugserver、
  WebInspector、XCTest runner、WebDriverAgent 辅助、可访问性审计、抓包、
  符号下载。
- **多语言消费者**——纯 Rust 库（`ios-core`）、PyO3 Python 模块（`ios_rs`）、
  C FFI（`ios-ffi`）共用同一份实现。

## 工作区结构

| Crate    | 发布产物                                            | 用途                                                                |
| -------- | --------------------------------------------------- | ------------------------------------------------------------------- |
| `ios-core` | crates.io                                          | 库：发现、配对、lockdown、隧道、RSD/XPC 与服务客户端                |
| `ios-cli`  | crates.io · 预编译 `ios` 二进制                    | 终端用户 CLI 工具                                                   |
| `ios-py`   | PyPI 包名 `rust-ios-device-tunnel`（导入名 `ios_rs`） | PyO3 Python 绑定：设备列表与隧道工作流                              |
| `ios-ffi`  | 预编译 `cdylib` + `staticlib` + `ios_rs.h`         | 给非 Rust 消费者使用的 C ABI                                         |

详细文档位于 [`docs/`](docs/)：[架构](docs/architecture.md)、
[构建](docs/build.md)、[feature flags](docs/features.md)、
[用法](docs/usage.md)、[CLI 对照](docs/cli-map.md)、
[隧道](docs/tunnel.md)、[协议](docs/protocol.md)、
[Python 绑定](docs/python-binding.md)、[故障排查](docs/troubleshooting.md)。

## 安装

### 预编译 CLI 二进制

到 [Releases 页面](https://github.com/oslo254804746/rust-ios-device/releases)
下载最新版本的 `ios-<version>-<target>.{tar.gz,zip}`。每个发布版本包含以下
target：

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

同名的 `ios-ffi-*` 归档包含 FFI 库文件和 `ios_rs.h` 头文件。每个产物都附带
`.sha256` 校验文件。

### 通过 crates.io

```sh
cargo install ios-cli            # 安装 `ios` 二进制
```

```toml
# Cargo.toml — 引用库
[dependencies]
ios-core = { version = "0.1.10", features = ["classic"] }
```

### Python

```sh
pip install rust-ios-device-tunnel    # 导入名为 `ios_rs`
```

## 快速开始

```sh
ios list                                       # 已连接设备（USB + 网络）
ios info                                       # 默认设备信息摘要
ios -u <UDID> lockdown get --key ProductVersion
ios syslog                                     # 实时设备日志
ios screenshot --output screenshot.png
ios tunnel start --userspace                   # iOS 17+ CoreDevice 隧道
```

需要选择设备且省略 `-u/--udid` 时，CLI 会使用 `ios list` 返回的第一台设备。
传入 `-u <UDID>` 或设置 `IOS_UDID` 可锁定某台设备。大多数命令默认输出 JSON，
便于脚本解析；传 `--no-json` 可获得人类友好的表格输出。

查看每个命令组：

```sh
ios --help
ios apps --help
ios file --help
ios instruments --help
ios tunnel --help
ios prepare --help
```

## 能力矩阵

CLI 的命令组与 `ios-core` 的服务模块基本一一对应。下表也列出了 [go-ios] 与
[pymobiledevice3] 的近似命令族，便于熟悉这两个项目的用户做对照。

| 领域                    | `ios` 命令                                                                                  | go-ios / pymobiledevice3 对照                                              |
| ---------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 发现与配对              | `list`, `listen`, `discover`, `pair`, `lockdown`                                            | go-ios `list`/`listen`/`pair`；pmd3 `usbmux`/`lockdown`/`bonjour`           |
| 设备信息与设置          | `info`, `mobilegestalt`, `diskspace`, `batterycheck`, `batteryregistry`, `activation`, `amfi` | go-ios `info`/`mobilegestalt`；pmd3 `lockdown`/`amfi`/`activation`         |
| 文件与容器              | `file`（AFC、应用、CoreDevice）、`crash`、`file-relay`                                       | go-ios `fsync`/`crash`；pmd3 `afc`/`crash`                                  |
| 应用与 UI 测试          | `apps`, `runtest`, `runwda`, `wda`, `springboard`                                           | go-ios `apps`/`install`/`launch`/`runtest`/`runwda`；pmd3 `apps`/dvt        |
| 诊断与日志              | `syslog`, `diagnostics`, `os-trace`, `notify`, `pcap`, `btlogger`                           | go-ios `syslog`/`diagnostics`/`pcap`；pmd3 `syslog`/`diagnostics`/`pcap`/`btlogger` |
| 开发者服务              | `instruments`, `debugserver`, `debug`, `ddi`, `symbols`, `accessibility-audit`, `webinspector`, `devicestate`, `memlimitoff` | go-ios `instruments`/`debug`/`image`/`ax`；pmd3 `developer dvt`/`mounter`/`webinspector` |
| iOS 17+ 传输            | `tunnel`, `rsd`, `forward`, `dproxy`                                                        | go-ios `tunnel`/`rsd`/`forward`；pmd3 RemoteXPC/tunnel                     |
| 设备剪贴板              | `pasteboard get`、`pasteboard set TEXT`、`pasteboard set --url URL`                          | go-ios `pasteboard`；pmd3 CoreDevice `paste`/`copy`                     |
| CoreDevice 配置         | `device-control configuration get|set ...`                                                      | pmd3 CoreDevice configuration actions                                     |
| CoreDevice 旋转         | `device-control orientation [left|right]`                                                        | pmd3 CoreDevice `rotate [left|right]`                                     |
| 管理与监督              | `profiles`, `provisioning`, `prepare`, `httpproxy`, `mdm`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` | go-ios `profile`/`prepare`/`httpproxy`/`mdm`/`erase`；pmd3 `profile`/`provision`/`restore` |
| 备份、定位与屏幕        | `backup`, `location`, `screenshot`, `notify`                                                | go-ios / pmd3 `backup`/`location`/`screenshot`                              |

按任务组织的示例见 [`docs/usage.md`](docs/usage.md)；与 go-ios / pmd3 命令族
的并排对照见 [`docs/cli-map.md`](docs/cli-map.md)。

## CoreDevice / iOS 17+ 隧道

iOS 17+ 工作流通过 CoreDevice 隧道与每台设备的 RSD 服务目录路由。能否使用某项
具体功能取决于设备暴露的服务面，而不是仅看 iOS 版本。

```sh
# 启动单个隧道（默认 = 用户态模式）
ios tunnel start --userspace

# 运行本地隧道管理器 HTTP 服务（go-ios 兼容 JSON 字段）
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

用户态隧道暴露一个本地 TCP 代理：客户端连接代理后，先发送 16 字节 IPv6 地址，
再发送 4 字节小端序端口号，然后开始转发数据。内核 TUN 模式也可用，但通常需要
管理员/root 权限。

在判断是否为实现 bug 之前，先确认设备实际暴露了哪些服务：

```sh
ios rsd services --all
ios rsd services --all --features
ios rsd check com.apple.coredevice.fileservice.control
ios file --coredevice --domain temporary ls /
```

RSD 服务列表默认以 JSON 输出，并保持旧的 `name`/`port` 项结构。传入 `--features` 后，
JSON 或配合 `--no-json` 的可读文本才会加入设备公布的 `features`；列表仍按服务名排序。
请求 feature 信息时，缺少列表会输出 `[]`，表示设备没有公布能力元数据，并不表示所有
操作都不支持。`rsd check` 也遵循相同的显式开关规则。

隧道/RSD/XPC 的 TCP 连接与初始协议建立现在有 15 秒上限；基于 TCP 的远程配对与
lockdown 建立也使用相同上限。遇到失效的隧道路由时会尽快返回超时，而不会等待主机
操作系统漫长的 SYN 重试窗口。后续服务请求仍遵循各自的超时行为。

如果设备未暴露所需 CoreDevice 服务（例如 fileservice 的 control/data 对），CLI
会报告清晰的缺失服务错误，而不是回退到别的服务名。完整的隧道生命周期见
[`docs/tunnel.md`](docs/tunnel.md)。

## Rust 库用法

`ios-core` **默认不启用任何具体服务 feature**。请按需启用，或使用分组 feature：

```toml
[dependencies]
ios-core = { version = "0.1.10", features = ["afc", "syslog"] }
```

| 分组         | 包含内容                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| `classic`    | afc, apps, crashreport, diagnostics, file_relay, heartbeat, house_arrest, installation, mcinstall, mobileactivation, notificationproxy, profiles, screenshot, springboard, syslog |
| `developer`  | accessibility_audit, amfi, btlogger, debugserver, dproxy, dtx, fetchsymbols, imagemounter, instruments, pcap, testmanager, webinspector |
| `management` | arbitration, companion, idam, misagent, power_assertion, preboard, prepare, restore                   |
| `ios17`      | apps, configuration, deviceinfo, diagnosticsservice, dproxy, fileservice, instruments, orientation, pasteboard, testmanager, mdns, tunnel-userspace |
| `full`       | classic + developer + ios17 + management + ostrace + supervised-pair + tunnel-kernel + backup2-manifest |

CLI 使用 `full`；库消费者通常应选择更窄的子集。

```rust
use ios_core::{ConnectOptions, list_devices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = list_devices().await?;
    let Some(device) = devices.first() else {
        println!("no device found");
        return Ok(());
    };

    let connected = ios_core::connect(
        &device.udid,
        ConnectOptions { skip_tunnel: true, ..Default::default() },
    )
    .await?;

    let version = connected.product_version().await?;
    println!("{} runs iOS {}", connected.info.udid, version);
    Ok(())
}
```

需要更底层的访问时，可使用 crate 根部重新导出的模块，例如 `ios_core::mux`、
`ios_core::lockdown`、`ios_core::xpc`，以及对应 feature 启用后的服务模块
`ios_core::afc`、`ios_core::apps`、`ios_core::syslog` 等。

## Python 绑定

```sh
pip install rust-ios-device-tunnel
```

或在本地 checkout 中构建：

```sh
cd crates/ios-py
uvx maturin develop
```

```python
import ios_rs

devices = ios_rs.list_devices()
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
print(tunnel.services)
print(tunnel.service_ports)    # 服务名 -> 设备端口
print(tunnel.service_features) # 服务名 -> 设备公布的标识符（可能为空列表）
print(tunnel.connect_info())

with tunnel.asyncio_proxy():
    # asyncio.open_connection() 在 with 作用域内会自动通过本地用户态代理
    # 路由到设备隧道地址。
    ...

tunnel.close()
```

`crates/ios-py/examples/pymobiledevice3_coredevice_bridge.py` 展示了如何让
pymobiledevice3 的 RemoteXPC 代码跑在 Rust 用户态隧道之上。

## C FFI

构建 C 兼容库与头文件：

```sh
cargo build --release -p ios-ffi
```

输出包括 `libios_ffi.{so,dylib,a}`（Windows 上是 `ios_ffi.dll` + `.lib`）以及
`crates/ios-ffi/include/ios_rs.h`。FFI 表面覆盖设备列表、lockdown 查询、
配对/服务访问与隧道生命周期。`ios_tunnel_rsd_services_json` 返回稳定排序的紧凑
JSON 服务映射，每个服务值包含 `port` 与 `features`。每个发布版本都为支持的 target
附带预编译归档。

## 从源码构建

### 环境要求

- Rust **1.80+**（工作区 MSRV）。
- 主机 usbmux 支持：
  - **macOS** —— 通常 Xcode/Finder 提供的 Apple 设备支持已经够用。
  - **Linux** —— 需要运行 `usbmuxd`，并配置 udev 权限。
  - **Windows** —— 通过 iTunes 或 Apple Devices 安装 Apple Mobile Device Support。
- Linux 需要 OpenSSL 开发文件（`libssl-dev`、`pkg-config`）。
- Windows 通过 vcpkg 静态链接 OpenSSL（`x64-windows-static-md`），需要设置
  `VCPKG_ROOT`、`VCPKGRS_TRIPLET`、`OPENSSL_STATIC=1`。
- 在本机测试 `ios-py` 时需要 Python 3.9+ 开发文件；构建 wheel/开发版本还需要 `maturin`。

### 常用命令

```sh
# 原生 crate 的工作区构建
cargo build --workspace --exclude ios-py

# 发布版 CLI 二进制
cargo build --release -p ios-cli

# 测试
cargo test --workspace --exclude ios-core --exclude ios-py
cargo test -p ios-core --all-features
# Python binding 本机测试（需要 Python 开发文件）
PYO3_PYTHON=/path/to/python cargo test -p ios-py

# Lint / 格式
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

从 checkout 直接运行 CLI：

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- --help
```

## 示例

CLI crate 包含可运行的 Rust 示例：

```sh
cargo run -p ios-cli --example device_info     -- <UDID>
cargo run -p ios-cli --example app_list        -- <UDID>
cargo run -p ios-cli --example file_transfer   -- <UDID>
cargo run -p ios-cli --example screenshot      -- <UDID>
cargo run -p ios-cli --example syslog_stream   -- <UDID>
cargo run -p ios-cli --example instruments_cpu -- <UDID>
cargo run -p ios-cli --example afc_debug       -- <UDID>
```

部分示例需要额外参数（路径、bundle ID 等），请先看源码或加 `--help`。

## 故障排查

- **设备不可见** —— 解锁设备、信任主机、重新插拔 USB，并确认 usbmuxd / Apple
  Mobile Device Support 在运行。
- **配对失败** —— 在确认影响后再删除旧的 pair record，然后从已解锁的设备重新
  配对。
- **旧设备隧道失败** —— 设备可能未暴露 CoreDevice tunnel/RSD；回退到 lockdown /
  usbmux 服务路径。
- **内核隧道失败** —— 改用用户态模式，或以创建 TUN 接口所需的权限运行。
- **开发者服务失败** —— 启用 Developer Mode，并按需挂载兼容的开发者磁盘镜像
  （`ios ddi`）。
- **CoreDevice fileservice 不可用** —— 用 `ios rsd services --all` 确认
  `com.apple.coredevice.fileservice.control` 与 `.data` 是否在列。缺失是设备侧
  服务面问题，而不是客户端 bug。

更多内容见 [`docs/troubleshooting.md`](docs/troubleshooting.md)。

## 安全与限制

- 这**不是 Apple 官方支持的 SDK**，不能替代 Xcode、Finder、Apple Configurator
  或官方 MDM 工具。
- 并非每个命令都在所有 iOS 版本、主机系统或配对状态下验证过；部分高级命令应
  视为协议实验。
- 修改设备状态的命令——`erase`、`restore`、`prepare`、`httpproxy`、`location`、
  `preboard`、描述文件安装/删除、备份恢复——都可能造成破坏。请先看 `--help`，
  优先在测试设备上使用。
- pair record 与监督证书是敏感凭据。请勿提交到仓库或写入共享日志。

## 贡献

欢迎贡献。开发环境配置、测试要求与 PR 指南见 [CONTRIBUTING.md](CONTRIBUTING.md)。
Bug 报告与功能请求模板位于 [`.github/ISSUE_TEMPLATE`](.github/ISSUE_TEMPLATE)。

## 安全报告

请通过私有渠道报告漏洞。详见 [SECURITY.md](SECURITY.md)。

## 许可证

可任选以下许可证之一使用：

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

## 致谢

本项目受到更广泛的 iOS 设备工具生态启发，尤其是：

- [go-ios](https://github.com/danielpaulus/go-ios)
- [pymobiledevice3](https://github.com/doronz88/pymobiledevice3)

兼容性仅在本仓库代码与测试支持的范围内实现。

[go-ios]: https://github.com/danielpaulus/go-ios
[pymobiledevice3]: https://github.com/doronz88/pymobiledevice3
