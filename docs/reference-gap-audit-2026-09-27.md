# 参考项目功能差异核对（2026-09-27）

本次从本地参考基线（pymobiledevice3：2026-08-29；go-ios：2026-08-27）
fetch 到最新远端引用，检查期间分别新增 184 和 4 个非合并提交。
参考工作区保持原状，直接读取远端 Git 对象；精确提交哈希和提交列表保存在
仓库外 `/tmp/rust-ios-device-reference-audit-20260927.json`。
以下结论来自源码和主机协议测试，未在本轮执行真机操作。

## 本分支已实现

| 功能 | 原有差距 | 本次行为 |
| --- | --- | --- |
| Display 停流 | 用客户端 UUID 构造旧 StopRequest；复用启动通道 | 使用 `stopAll` / UInt32 `identifiers`，后续 Display 请求打开新连接，保留原隧道路由。旧 UUID API 和 session 停止均停止全部媒体流 |
| Display 协商与错误 | AVC offer 固定 `VRAE:0`；9022 只是通用报错 | 移除禁止分辨率适配的 token；返回 `DisplayError::MediaInUse`，提示关闭占用摄像头／麦克风的应用 |
| 剪贴板监听 | 只有实验性 AUTONOTIFY/PUSH，空闲超时退出 | 默认通过 RSD notification-proxy shim 监听 Darwin 通知；每次 PULL 建立新连接，忽略初始快照，按 nonce 和计数去重，空闲持续等待 |
| 剪贴板读取策略 | CLI 默认解析全部表示，富文本／图片可能拖住 daemon | CLI `get` / `watch` 默认 `promisesecondary`；显式 `--policy resolved` 保留完整读取；Rust 旧 get API 默认不变 |
| 辅助功能检查清理 | 焦点命令结束后没有统一关闭监控 | 正常结束、错误和 Ctrl-C 都尝试隐藏 inspector 并关闭 app monitoring；清理有超时，保留原始操作错误 |
| 设备准备 | 默认跳过列表缺少新增设置页 | `DEFAULT_SKIP_SETUP_KEYS` 增加 `LiquidGlass` |
| 现代 RSD 身份与握手 | 缺主机统一 UUID 和主动 Handshake | 使用 macOS remoted 本机 UUID 或 OS hostname 的 UUIDv3；直连和 userspace 共用主动握手及有期限的新连接回退 |
| iOS 27 Cryptex DDI | 只有普通 image mounter 流程 | 新 Cryptex 查询、nonce handle、Cryptex1 TSS、五文件流式安装及安装后确认；`ddi auto/mount/status/unmount` 按 iOS 27 门槛选择，已有镜像明确报冲突 |
| Wi-Fi 身份识别 | 缺 authTag / altIRK、私有 MAC 和未知 UDID 候选验证 | 使用既有记录匹配 mobdev2 HostID tag 和 RemotePairing altIRK；私有 MAC 通过有期限的 lockdown 会话验证 UniqueDeviceID；显式配对保留 altIRK |
| 主机 doctor | 只有排障文档 | 新 `ios doctor` JSON／人类报告；检查 usbmuxd、IPv6、TUN、本地 manager 和可选 Bonjour；未测试的设备能力明确为未知 |

Display 捕获的 Ctrl-C 和输出文件打开失败路径也已调整，避免跳过已有的显式停流。
自定义 XPC 字节流若要重复执行 Display 请求，须配置
`XpcClient::with_reconnector`；未配置时明确报错，不复用旧回复通道。

参考协议位置：
[pmd3 Display](https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/core_device/display_service.py)、
[媒体协商](https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/core_device/media_stream_offer.py)、
[剪贴板](https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/core_device/pasteboard_service.py)、
[go-ios 辅助功能](https://github.com/danielpaulus/go-ios/blob/main/ios/accessibility/accessibility_control.go)、
[设备准备](https://github.com/danielpaulus/go-ios/blob/main/ios/mcinstall/prepare.go)。

## 仍未跟进或仅部分覆盖

| 优先级 | 上游近期变化 | 当前覆盖与后续工作 |
| --- | --- | --- |
| 真机待验 | iOS 27.2 RSD host-wide UUID、macOS remoted 身份 | 主机协议实现已完成，仍需 macOS remoted 共存与 iOS 27.2 跨进程重连验证；hostname 变更会改变 fallback UUID |
| 真机待验 | iOS 27 Cryptex DDI 自动安装 | 协议链和模拟服务测试已完成；尚未实际调用 Apple TSS 或安装设备。未验证的 roll-nonce 没有暴露命令；见 [Cryptex DDI](cryptex-ddi.md) |
| 部分 | Wi-Fi mobdev2 / RemotePairing authTag 与私有 MAC 识别 | 算法和候选流程已完成；只读既有文件记录，尚无 usbmuxd-only 记录枚举；真实 TLS、私有 MAC 轮换、scoped IPv6 与仅 Wi-Fi 连接待验。旧 remote 记录缺 altIRK 时仍走 direct RSD；见 [Wi-Fi discovery](wifi-discovery.md) |
| 中 | CDP 的 Playwright/Puppeteer、JSContext、iframe、Fetch、Memory/Performance 兼容 | 已有页面 CDP/WebDriver 桥；没有上游近期完整 browser-endpoint 会话及各域兼容层。需按协议域实现并引入客户端黄金流程测试，不能宣称整个 CDP 缺失或完整兼容 |
| 中 | tunneld `/connect` WebSocket 与 federation | 已有本地 tunnel manager 和 TCP proxy；缺 HTTP 隧道桥、远端 federation 及 hop budget |
| 中 | restore preflight requests、TSS batch 和固件消息处理 | 已有 `restore preflight-info` 元数据查询；缺设备侧预检请求生成、批量 TSS riders 与新增恢复消息链路 |
| 中 | serve-web / VNC、图像剪贴板双向同步 | 已有编码后的 RTP 捕获和 HID；缺完整解码查看器、Web/VNC 服务、双向剪贴板和回环去重。新 watch 仅报告设备变化；断线自动重连仍未实现 |
| 部分 | `doctor` 主机能力诊断 | 主机基础报告已完成；尚未实现 OS USB 硬件枚举、设备信任／Developer Mode／DDI 探测或 native remoted 生命周期诊断 |
| 待专项核对 | legacy CopyDevices 断连、WebInspector 握手重试、native tunnel 生命周期、XDG 目录 | 属于平台／传输差异，未在本批修改；需各自的可复现协议或平台测试 |

go-ios 本期新增的 Display 启流和 HID touch 在 Rust 中已有对应实现，本次修复了
相关媒体生命周期；这不表示两者在所有 iOS 版本上已完成真机等价验证。
pmd3 本期大消息分帧、设备路径约束与帧长度限制，在 Rust 中已有对应的分帧、
路径与预算机制，不能仅凭上游提交标题判为缺失；完整逐项安全审计不在本批结论内。

后续依据：
[RemoteXPC](https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/remotexpc.py)、
[mounter](https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/cli/mounter.py)、
[lockdown](https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/lockdown.py)、
[WebInspector](https://github.com/doronz88/pymobiledevice3/tree/master/pymobiledevice3/services/web_protocol)、
[上游近期提交](https://github.com/doronz88/pymobiledevice3/commits/master/)。

## 验证边界

新增主机测试覆盖 Display 新连接与 StopRequest、9022 映射、协商 token，
剪贴板通知与去重／取消／错误，以及辅助功能关闭消息与 CLI 参数。
首批 Linux 主机验证结果（Display／剪贴板／辅助功能）：

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --all-features`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace --all-features`：PyO3 `extension-module` 模式的
  测试二进制出现 Python 符号未链接，未通过该组合；按仓库 CI 拆分验证。
- `cargo test --workspace --exclude ios-py --all-features`：1454 通过，
  0 失败，1 项既有测试忽略。
- `cargo test -p ios-py --no-default-features`：6 通过，0 失败；随后在 uv
  创建的根目录 `.venv`（CPython 3.12.13）中重新验证，同样 6 通过。
  uv 管理的 libpython 和标准库路径通过单次命令的 `LD_LIBRARY_PATH` /
  `PYTHONHOME` 提供。
- 在该 venv 中执行 `uvx maturin develop`：abi3 wheel 构建和 editable 安装
  通过；`import ios_rs`、包版本、公开 API 和无效模式的 `ValueError` 合同通过。
  虚拟环境不入库，复现步骤见 [Python 构建说明](python-binding.md)。
- `cargo check -p ios-core --no-default-features --features pasteboard`：通过；
  该最小组合仍有两条未使用隧道字段／方法的 dead-code 警告。

仍需真机验证：媒体停止后摄像头／麦克风恢复、重复启停、HID 结束后的清理，
Notes 等应用复制富内容时的监听，以及监督设备上 LiquidGlass 页跳过行为。
新分支仍未包含剪贴板自动重连、双向剪贴板或完整 CDP 实现。
第一阶段的实施／验证状态见 [实施计划](implementation-plan-2026-09-27.md)。
