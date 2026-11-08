# rust-ios-device

[English](README.md) | 简体中文

一组用于操作真实 iOS 设备的 Rust 库、语言绑定和 `ios` 命令行工具，通过
usbmuxd、lockdown、CoreDevice/RemoteXPC 与 Apple 设备服务与设备通信，全面支持
**iOS 15.x – iOS 27.2**。

[![Crates.io — ios-core](https://img.shields.io/crates/v/ios-core.svg?label=ios-core)](https://crates.io/crates/ios-core)
[![Crates.io — ios-cli](https://img.shields.io/crates/v/ios-cli.svg?label=ios-cli)](https://crates.io/crates/ios-cli)
[![PyPI — rust-ios-device-tunnel](https://img.shields.io/pypi/v/rust-ios-device-tunnel.svg?label=rust-ios-device-tunnel)](https://pypi.org/project/rust-ios-device-tunnel/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可证)
[![MSRV](https://img.shields.io/badge/MSRV-1.80-orange.svg)](#环境要求)

> **状态：实验性（`v0.1.16`）。** 项目能力面覆盖广泛，已在 **iOS 15.8.6（Legacy）**、
> **iOS 18.6 / 18.7（Modern）** 与 **iOS 27.2（Modern）** 真实设备上完成端到端验证，
> 但 1.0 稳定版本发布前公开 API 与 CLI 仍可能调整。具体服务是否可用取决于 iOS 版本、
> 设备信任与配对状态、Developer Mode、监督状态以及主机的 Apple Mobile Device 组件。

[能力清单](docs/capabilities.md) 分别登记各命令/服务的实现、主机测试、模拟流程和
真机证据；具体支持边界以该清单和关联专题文档为准。

## 亮点

- **广泛的 iOS 版本支持：iOS 15.x – iOS 27.2**——涵盖 iOS 15.8.6 Legacy（usbmuxd /
  Lockdown / DDI `.dmg`）、iOS 18.6/18.7 Modern 以及 iOS 27.2 Modern
  （CoreDevice / RemoteXPC / 个性化 Cryptex DDI）。
- **跨平台 CLI（`ios`）**——60+ 子命令，覆盖设备管理、文件传输、应用生命周期、
  Instruments 性能采样、调试、音视频流抓取、HID 自动化输入、WebInspector、描述文件、
  恢复、监督和隧道管理。
- **CoreDevice 隧道（iOS 17–27+）**——支持内核 TUN 与无需管理员权限的**用户态隧道**
  （`--userspace`）、`tunnel start`、`tunnel serve`，内置 **20ms ICMPv6 Echo
  Request Nudge** 低延迟突发防抖机制，并同时支持 TCP 流代理与 IPv6 UDP 数据报转发。
- **屏幕与媒体流（`device-control display` 与 `viewer`）**——
  - `ios device-control display status / video / audio`：支持 iOS 27+
    `dtremotedisplayd` 实时 **H.265 (HEVC)** 视频与 **AAC** 音频 RTP 访问单元抓取
    （可在无管理员权限的 Userspace Tunnel 下直接运行）。
  - `ios viewer web` 与 `ios viewer vnc`：交互式 Web 查看器（MJPEG 流、PNG 帧、
    触控/按键/物理按钮/屏幕旋转控制）与 RFB 3.8 VNC 服务端，支持双向剪贴板同步
    （`--share-clipboard`）与 Pro Max 16-bit Display P3 截图色彩解码。
- **通用 HID 输入（`ios hid`）**——支持 `tap`、`swipe`、`button`、分阶段 `touch`、
  5 指多点触控 `multitouch`（`--points`）、`text` 与 `key`，内置 500ms 触控授权
  稳定等待、300ms 虚拟键盘弹出等待以及大写字母/符号的 Shift 修饰键前置/后置序列。
- **WebInspector、CDP 与 WebDriver（`ios webinspector`）**——提供 `opened-tabs`、
  `eval`、`launch`、`js-shell`、`cdp`（兼容 `puppeteer-core` 与 `playwright-core`
  的 `connectOverCDP`，支持 `Runtime`、`Page`、`Network`、`Fetch`、`Input`、
  `Performance`、`HeapProfiler` 域）以及 W3C `webdriver`（兼容
  `selenium-webdriver`）。
- **文件管理（`ios file` 与 CoreDevice `fileservice`）**——完整的 AFC 客户端，支持
  全部扩展操作码（`connection-info`、带范围的 `hash`、`pread`、`pwrite`、
  `truncate`、`set-time`、`dir-size`）以及 iOS 17+ CoreDevice `fileservice`
  （`--coredevice` 下的 `ls`、`mkdir`、`push`、`pull`、`mv`、`rm --recursive`）。
- **开发者与诊断套件**——Cryptex DDI 自动下载挂载（`ddi auto`、`ddi cryptex`）、
  Instruments（`ps`、`apps`、`cpu`、`sysmon-process`、`gpu`、`fps`、`kdebug`）、
  CoreProfileSession Stackshot（`sharedCache` UUID 提取）、`accessibility-audit`
  （`axAudit` DTX）、`debug` 与 `debugserver send`（支持 iOS 17+ RSD `debugproxy`
  自动回退）、`symbols`、`devicestate`、`memlimitoff`、`pcap --interface`、
  `btlogger`（`.pklg`/`.pcapng`）、`pasteboard` 以及 `device-control configuration`
  （包含 iOS 26/27 新增的 `liquid-glass-opacity` 与 `increase-contrast`）。
- **多语言消费者**——纯 Rust 库（`ios-core`）、PyO3 Python 模块（`ios_rs`）、
  C FFI（`ios-ffi`）共用同一份底层实现。

## 工作区结构

| Crate    | 发布产物                                            | 用途                                                                |
| -------- | --------------------------------------------------- | ------------------------------------------------------------------- |
| `ios-core` | crates.io                                          | 库：发现、配对、lockdown、隧道、RSD/XPC 与服务客户端                |
| `ios-cli`  | crates.io · 预编译 `ios` 二进制                    | 终端用户 CLI 工具                                                   |
| `ios-py`   | PyPI 包名 `rust-ios-device-tunnel`（导入名 `ios_rs`） | PyO3 Python 绑定：设备列表、受控会话与隧道工作流                    |
| `ios-ffi`  | 预编译 `cdylib` + `staticlib` + `ios_rs.h`         | 给非 Rust 消费者使用的 C ABI                                         |

详细文档位于 [`docs/`](docs/)：[架构](docs/architecture.md)、
[构建](docs/build.md)、[feature flags](docs/features.md)、
[用法](docs/usage.md)、[CLI 对照](docs/cli-map.md)、
[隧道](docs/tunnel.md)、[查看器](docs/viewer.md)、[协议](docs/protocol.md)、
[Python 绑定](docs/python-binding.md)、[故障排查](docs/troubleshooting.md)、
[宿主输出与取消合同](docs/output-contract.md)。
[公共兼容规则](docs/public-contract.md) 定义 Rust/CLI/C/Python 的维护候选接口和
迁移方式；主机兼容 fixture 不代表稳定真机工作流已认证。
[plist/JSON 结果合同](docs/plist-json-contract.md) 登记保留的编码和 stdout 失败退出码：
结果、帮助或版本写入失败（包括管道关闭）返回 1，成功输出格式保持兼容。

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
ios-core = { version = "0.1.16", features = ["classic"] }
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
ios tunnel start --userspace                   # iOS 17–27+ CoreDevice 隧道
```

需要选择设备且省略 `-u/--udid` 时，CLI 会使用 `ios list` 返回的第一台设备。
传入 `-u <UDID>` 或设置 `IOS_UDID` 可锁定某台设备。大多数命令默认输出 JSON，
便于脚本解析；传 `--no-json` 可获得人类友好的表格输出。

### 核心功能速览

```sh
# 屏幕与媒体流（iOS 27+ H.265/AAC RTP 抓取 & 交互式 Web/VNC 查看器）
ios -u <UDID> device-control display video --output capture.h265 --duration-ms 5000
ios -u <UDID> device-control display audio --output capture.aac --duration-ms 5000
ios -u <UDID> viewer web --listen 127.0.0.1:8090 --share-clipboard
ios -u <UDID> viewer vnc --listen 127.0.0.1:5901 --share-clipboard

# 通用 HID 输入（单指点击、滑动、5 指多点触控、带 Shift 修饰序列的文本输入）
ios -u <UDID> hid --confirm tap 0.5 0.5
ios -u <UDID> hid --confirm swipe 0.5 0.8 0.5 0.2 --duration-ms 300
ios -u <UDID> hid --confirm multitouch --points "1:0.20:0.40,2:0.35:0.35,3:0.50:0.30,4:0.65:0.35,5:0.80:0.40"
ios -u <UDID> hid --confirm text "Hello, iOS 27!"

# WebInspector、CDP（Puppeteer/Playwright）与 W3C WebDriver（Selenium）
ios -u <UDID> webinspector opened-tabs
ios -u <UDID> webinspector cdp --host 127.0.0.1 --port 9222 --allow-loopback-without-auth
ios -u <UDID> webinspector webdriver --host 127.0.0.1 --port 4444 --allow-loopback-without-auth

# 文件管理（AFC 扩展操作码 & iOS 17+ CoreDevice fileservice）
ios -u <UDID> file hash /Books/Purchases/sample.plist --offset 0 --length 64
ios -u <UDID> file --coredevice --domain temporary ls /

# 开发者与诊断套件
ios -u <UDID> ddi auto
ios -u <UDID> instruments fps --duration 5
ios -u <UDID> debugserver send --command qSupported
ios -u <UDID> device-control configuration set liquid-glass-opacity --mode tinted
```

## 能力矩阵

CLI 的命令组与 `ios-core` 的服务模块基本一一对应。下表也列出了 [go-ios] 与
[pymobiledevice3] 的近似命令族，便于熟悉这两个项目的用户做对照。

| 领域                    | `ios` 命令                                                                                  | go-ios / pymobiledevice3 对照                                              |
| ---------------------- | ------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 发现与配对              | `list`, `listen`, `discover`, `pair`, `lockdown`, `doctor`                                  | go-ios `list`/`listen`/`pair`；pmd3 `usbmux`/`lockdown`/`bonjour`           |
| 设备信息与设置          | `info`, `mobilegestalt`, `diskspace`, `batterycheck`, `batteryregistry`, `activation`（state、session-info、info、activate、deactivate、itunes-activate）, `amfi` | go-ios `info`/`mobilegestalt`；pmd3 `lockdown`/`amfi`/`activation`         |
| 文件与容器              | `file`（AFC 与扩展操作码 `connection-info`/`hash`/`pread`/`pwrite`/`truncate`/`set-time`/`dir-size`、`--app` House Arrest、`--coredevice` `fileservice`）、`crash`、`file-relay` | go-ios `fsync`/`crash`；pmd3 `afc`/`crash`/CoreDevice `fileservice`         |
| 应用与 UI 测试          | `apps`（含 iOS 17+ `install-record`）, `runtest`, `runxctest`, `runwda`, `wda`, `springboard` | go-ios `apps`/`install`/`launch`/`runtest`/`runwda`；pmd3 `apps`/dvt        |
| 诊断与日志              | `syslog`, `diagnostics`（reboot/shutdown）, `os-trace`（含 archive/collect）, `notify`, `pcap`（`--interface`）, `ip`, `btlogger`（`.pklg`/`.pcapng`） | go-ios `syslog`/`diagnostics`/`pcap`；pmd3 `syslog`/`diagnostics`/`pcap`/`btlogger` |
| 开发者服务              | `instruments`（`ps`、`apps`、`cpu`、`sysmon-process`、`gpu`、`fps`、`kdebug`）, `debugserver`（Lockdown 与 RSD `debugproxy` 自动回退）, `debug`, `ddi`（`auto`、`cryptex`）, `symbols`（CoreProfileSession Stackshot 与 RSD）, `accessibility-audit`, `webinspector`（`opened-tabs`、`eval`、`launch`、`js-shell`、`cdp`、`webdriver`）, `devicestate`, `memlimitoff` | go-ios `instruments`/`debug`/`image`/`ax`；pmd3 `developer dvt`/`mounter`/`webinspector` |
| iOS 17–27+ 传输         | `tunnel`, `rsd`, `forward`, `dproxy`                                                        | go-ios `tunnel`/`rsd`/`forward`；pmd3 RemoteXPC/tunnel                     |
| 设备剪贴板              | `pasteboard get\|set\|watch`（Darwin 通知监听、独立 PULL 连接、二进制 UTI 数据）；实验性 `resolve\|export --experimental` | go-ios `pasteboard`；pmd3 CoreDevice `paste`/`copy` 和剪贴板监听          |
| CoreDevice 配置         | `device-control configuration get\|set ...`（含 iOS 26/27 `liquid-glass-opacity` 与 `increase-contrast`） | pmd3 CoreDevice configuration actions                                     |
| CoreDevice 旋转         | `device-control orientation [left\|right]`                                                  | pmd3 CoreDevice `rotate [left\|right]`                                    |
| 通用 HID 输入           | `hid --confirm tap\|swipe\|button\|touch\|multitouch\|text\|key`（自动建立 Display/RTP 触控授权、5 指多点触控、Shift 修饰序列） | pmd3 CoreDevice HID button/touch/keyboard helpers                         |
| CoreDevice 显示媒体流   | `device-control display status\|video\|audio`（iOS 27+ `dtremotedisplayd` 实时 H.265/HEVC 视频与 AAC 音频 RTP 抓取，支持用户态或内核隧道） | pmd3 CoreDevice display/screen-stream helpers                             |
| 交互式查看器与 VNC      | `viewer web\|vnc\|clipboard`（MJPEG/PNG Web UI、RFB 3.8 VNC 服务端、16-bit Display P3 解码、原生 HID、`--share-clipboard`；详见 [viewer 文档](docs/viewer.md)） | CoreDevice screencapture/HID/pasteboard 组合工作流                         |
| 管理与监督              | `profiles`, `wifi`, `provisioning`, `prepare`, `httpproxy`, `mdm`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` | go-ios `profile`/`wifi`/`prepare`/`httpproxy`/`mdm`/`erase`；pmd3 `profile`/`provision`/`restore` |
| 备份、定位与屏幕        | `backup`, `location`, `screenshot`（PNG / MJPEG 流）, `notify`                              | go-ios / pmd3 `backup`/`location`/`screenshot`                              |

按任务组织的示例见 [`docs/usage.md`](docs/usage.md)；与 go-ios / pmd3 命令族
的并排对照见 [`docs/cli-map.md`](docs/cli-map.md)。

## CoreDevice / iOS 17–27+ 隧道

iOS 17–27+ 工作流通过 CoreDevice 隧道与每台设备的 RSD 服务目录路由。能否使用某项
具体功能取决于设备暴露的服务面，而不是仅看 iOS 版本。

```sh
# 启动单个隧道（默认 = 用户态模式）
ios tunnel start --userspace

# 运行本地隧道管理器 HTTP 服务（go-ios 兼容 JSON 字段）
ios tunnel serve --userspace --host 127.0.0.1 --port 49151 --auth-config PRIVATE_POLICY_FILE
```

- **用户态与内核模式**——用户态隧道无需管理员/root 权限即可运行，并在本地暴露代理端口：
  客户端先发送 16 字节 IPv6 地址与 4 字节小端序端口号，即可代理 TCP 流或转发 IPv6 UDP
  数据报（供 `dtremotedisplayd` H.265/AAC RTP 音视频流与 HID 触控授权使用）。内核 TUN
  模式（`--kernel`）同样支持。
- **20ms ICMPv6 Echo Request Nudge**——当活跃 TCP 流等待设备响应时，用户态 `smoltcp`
  协议栈会以 20ms 间隔发送 ICMPv6 Echo Request Nudge，确保 iOS 18–27+ 上的高频 HTTP/2
  RemoteXPC 与 DTX 突发流量及时刷新 `CDTunnel` 队列。
- **受控管理器策略**——`tunnel serve` 需要 scoped policy 或显式 loopback 授权，客户端通过
  `--token-file` 私密文件认证；创建、权限与重载见[管理访问配置](docs/management-access.md)。

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

隧道/RSD/XPC 的 TCP 连接与初始协议建立有 15 秒上限；基于 TCP 的远程配对与
lockdown 建立也使用相同上限。遇到失效的隧道路由时会尽快返回超时，而不会等待主机
操作系统漫长的 SYN 重试窗口。后续服务请求仍遵循各自的超时行为。

如果设备未暴露所需 CoreDevice 服务（例如 fileservice 的 control/data 对），CLI
会报告清晰的缺失服务错误，而不是回退到别的服务名。完整的隧道生命周期见
[`docs/tunnel.md`](docs/tunnel.md)。

## Rust 库用法

`ios-core` **默认不启用任何具体服务 feature**。请按需启用，或使用分组 feature：

```toml
[dependencies]
ios-core = { version = "0.1.16", features = ["afc", "syslog"] }
```

| 分组         | 包含内容                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| `classic`    | afc, apps, crashreport, diagnostics, file_relay, heartbeat, house_arrest, installation, mcinstall, mobileactivation, notificationproxy, profiles, screenshot, springboard, syslog |
| `developer`  | accessibility_audit, amfi, btlogger, cryptex, debugserver, dproxy, dtx, fetchsymbols, imagemounter, instruments, pcap, testmanager, webinspector |
| `management` | arbitration, companion, idam, misagent, power_assertion, preboard, prepare, restore                   |
| `ios17`      | apps, configuration, cryptex, deviceinfo, diagnosticsservice, dproxy, fileservice, iconservice, installcoordination, instruments, orientation, screencapture, hid, display, pasteboard, testmanager, mdns, tunnel-userspace |
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

需要更底层的访问时，可使用 crate 根部重新导出的公开类型 `ios_core::MuxClient`、
`ios_core::LockdownClient`、`ios_core::RsdHandshake` 和 `ios_core::XpcClient`
（需要 `tunnel`）。传输模块本身为私有实现；服务模块按对应 feature 启用，例如
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
if not devices:
    raise SystemExit("no device found")
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
try:
    print(tunnel.services)
    print(tunnel.service_ports)    # 服务名 -> 设备端口
    print(tunnel.service_features) # 服务名 -> 设备公布的标识符（可能为空列表）
    print(tunnel.connect_info())

    with tunnel.asyncio_proxy():
        # asyncio.open_connection() 在 with 作用域内会自动通过本地用户态代理
        # 路由到设备隧道地址。
        print("userspace proxy active")

finally:
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
`crates/ios-ffi/include/ios_rs.h`。FFI 表面覆盖设备列表、受控设备会话
（`IosDeviceSession`）、lockdown 查询、隧道生命周期、RSD 元数据、已安装应用列表、
AFC 文件操作与 syslog 读取。
`ios_tunnel_rsd_services_json` 返回稳定排序的紧凑 JSON 服务映射，每个服务值包含
`port` 与 `features`。每个发布版本都为支持的 target 附带预编译归档。

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

# 本地/CI 完整检查（Python 3.9+、Rust 1.80 工具链、uv、C 编译器）
python scripts/validate.py full --python /path/to/python
# 开发中的局部检查
python scripts/validate.py quick --package ios-core
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
- **内核隧道失败** —— 改用 `--userspace` 用户态模式，或以创建 TUN 接口所需的权限运行。
- **开发者服务失败** —— 启用 Developer Mode，并按需挂载兼容的开发者磁盘镜像
  （`ios ddi auto`）。
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
