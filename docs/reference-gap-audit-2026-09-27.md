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
| 高 | iOS 27.2 RSD host-wide UUID、macOS remoted 身份 | 当前有多种 RSD bootstrap，但不等同于新版主动 Handshake 和跨进程统一 UUID。需实现身份来源／持久化及 macOS remoted 兼容，并做跨进程重连真机回归 |
| 高 | iOS 27 Cryptex DDI 自动安装 | 当前为 image mounter/DDI 流程，缺 Cryptex 安装链路。上游先按 17.4 接入、随后收窄为 27，须采用最新版本门槛；不能把普通 DDI 成功当作 Cryptex 已就绪 |
| 高 | Wi-Fi mobdev2 / RemotePairing authTag 与私有 MAC 识别 | 已有 Bonjour 与配对发现；尚无新 authTag/altIRK 匹配和未知 UDID 的配对记录候选流程。需合成广播测试和仅 Wi-Fi 设备覆盖 |
| 中 | CDP 的 Playwright/Puppeteer、JSContext、iframe、Fetch、Memory/Performance 兼容 | 已有页面 CDP/WebDriver 桥；没有上游近期完整 browser-endpoint 会话及各域兼容层。需按协议域实现并引入客户端黄金流程测试，不能宣称整个 CDP 缺失或完整兼容 |
| 中 | tunneld `/connect` WebSocket 与 federation | 已有本地 tunnel manager 和 TCP proxy；缺 HTTP 隧道桥、远端 federation 及 hop budget |
| 中 | restore preflight requests、TSS batch 和固件消息处理 | 已有 `restore preflight-info` 元数据查询；缺设备侧预检请求生成、批量 TSS riders 与新增恢复消息链路 |
| 中 | serve-web / VNC、图像剪贴板双向同步 | 已有编码后的 RTP 捕获和 HID；缺完整解码查看器、Web/VNC 服务、双向剪贴板和回环去重。新 watch 仅报告设备变化；断线自动重连仍未实现 |
| 中 | `doctor` 主机能力诊断 | 有排障文档，缺系统 USB／daemon／网络能力的自动诊断报告 |
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
本次 Linux 主机验证结果：

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
新分支未包含自动重连、双向剪贴板、完整 CDP 或 Cryptex 实现。
