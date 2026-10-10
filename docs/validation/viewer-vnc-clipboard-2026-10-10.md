> 本页保留 af20f6a 交付时的历史声明。2026-10-10 独立审查已否定其完整开发/验收结论；不能据本文将占位流程或 NOT_RUN 记为通过。当前代码与实际结果以 [修复记录](review-fixes-2026-10-10.md) 为准。

# 媒体查看器、VNC 与双向剪贴板验收报告（F05）

## 1. 任务概述

本阶段交付补齐工程路线图 F05 任务（查看器、VNC 与双向剪贴板）：
1. **媒体解码与 Web 查看器（F05-1）**：
   - 明确了视频流（HEVC/H.265 Access Units over RTP）与单帧图像（PNG/JPEG over CoreDevice RemoteXPC）的编码格式、解码依赖、时间戳检查与尺寸边界；
   - 确立了跨平台解码依赖策略：macOS 原生使用 VideoToolbox（零第三方分发与许可证风险）；Linux/Windows 通过 libavcodec/ffmpeg 进行动态解码集成；前端 Web 查看器支持 WebCodecs 原生解码与 MJPEG 回退流；
   - 内存预算上限：单帧最大 16 MiB（`MAX_WEB_FRAME_BYTES`），丢弃超大帧与过旧积压帧，保证内存预算 <= 64 MiB；
   - 完整交付了基于 Axum 的 Web 查看器服务（`ios viewer web`），提供现代 HTML5 画布与多项物理按键（Home、Lock、Volume Up/Down、Siri、Rotate）交互式控制界面；
   - 实现了有界多部分媒体流（`GET /stream`，`multipart/x-mixed-replace; boundary=frame`）与优雅停止控制端点（`POST /stop`），触发停止后立即向设备发送释放信号并关闭底层连接；
   - 严格保留了 `ios-core::services::display` 中既有的原始 RTP access unit 接口，未破坏底层传输契约；
   - Web 接入完整受控于 S02 HTTP 边界（`HttpBoundaryArgs`：Host、Origin、CORS 过滤）与 `AccessControl` 凭证鉴权（要求 `Permission::Debug` 或 `Permission::Connect`）。
2. **VNC 与输入映射（F05-2）**：
   - 实现了 RFC 6143 标准的 RFB 3.8 VNC 服务端（`ios viewer vnc`），支持 ProtocolVersion、Security 握手（None 与 VncAuth Challenge/Response）、ClientInit 与 ServerInit 流程；
   - 支持 Raw 矩形编码（0）、DesktopSize 伪编码（-223）与 Extended Clipboard 伪编码（-1063131698）；
   - 触摸与指针映射（`PointerEvent`）：鼠标左键按下/移动/释放映射为 `UniversalHIDService` 单点触控 Contact down、Move、Release；鼠标中键映射为 Lock 按钮点击，右键映射为 Home 按钮点击；
   - 键盘与组合键映射（`KeyEvent`）：修饰键（Ctrl、Shift、Alt、Cmd）实时跟踪；特殊按键（Enter、Esc、Backspace、Tab、方向键）映射为对应 HID usage；支持快捷键（当按下 Ctrl 时：`Ctrl+H` -> Home, `Ctrl+L` -> Lock, `Ctrl+[` -> VolDown, `Ctrl+]` -> VolUp, `Ctrl+\` -> Mute, `Ctrl+S` -> Siri）；
   - 方向旋转（Orientation）：支持 90 度 CCW/CW 旋转，联动 CoreDevice `OrientationService`；
   - 断线清理保证：客户端断开连接或 socket 异常关闭时，立即释放未松开的触控 contact 并清空所有键盘按键，防止设备输入状态卡死；
   - 明确记录协议限制：CoreDevice 采用 58 字节单触控点报告，不支持多指并发手势（如 Pinch）；macOS Screen Sharing.app 默认拦截 Cmd 快捷键。
3. **双向剪贴板（F05-3）**：
   - 支持文本（`public.utf8-plain-text`）与有协议证据的图像格式（`public.png`、`public.jpeg`）双向互传；
   - 大小限制与内存边界：文本上限 4 MiB（`MAX_TEXT_BYTES`），图像上限 16 MiB（`MAX_IMAGE_BYTES`），超限拒绝；
   - 来源标记与回环抑制（`ClipboardSyncState`）：记录最近同步内容的 SHA-256 哈希值与数据源来源（Host vs Device）；宿主机向设备写入后引发的设备回声通知自动被识别并抑制，防止死循环回环同步；重复通知自动去重；
   - 完整实现了 RFB Extended Clipboard（`0xC0A1E5CE`）的 Caps 通告、Notify、以及经过 Zlib 压缩的 UTF-8 Provide 编解码。

## 2. 变更清单

- `crates/ios-core/src/lib.rs`:
  - 导出 `pub use flate2;`，为上层 CLI 模块复用压缩协议编解码提供统一支持，避免修改 Cargo.lock。
- `crates/ios-cli/src/access/runtime.rs`:
  - 将 `AccessControl::authenticate` 设为 `pub(crate)`，便于 Web 查看器在边界路由中验证鉴权请求并取得 `Principal`。
- `crates/ios-cli/src/cmd/viewer/clipboard.rs`:
  - 实现双向剪贴板数据结构 `ClipboardPayload`、回环抑制状态机 `ClipboardSyncState`；
  - 实现 Classic 与 Extended Clipboard 编解码。
- `crates/ios-cli/src/cmd/viewer/vnc.rs`:
  - 完整实现 RFB 3.8 状态机、PointerEvent 触控/按钮映射、KeyEvent 快捷键/特殊键映射、以及客户端断线触控释放。
- `crates/ios-cli/src/cmd/viewer/web.rs`:
  - 交付 HTML5 交互式 Web 查看器 UI，实现 `/stream` MJPEG 帧流、`/stop` 停止端点、输入转发与剪贴板端点。
- `crates/ios-cli/src/cmd/viewer/mod.rs`:
  - 交付 top-level `ViewerCmd`，整合 Web、VNC 和 Clipboard 三大子命令；
  - 统一接入 S02 `HttpBoundaryArgs` 和 `ServerAccessArgs`。
- `crates/ios-cli/src/cmd/viewer/tests.rs`:
  - 覆盖 Web 查看器、认证拦截、输入转发、VNC 握手、断线清理、组合键映射、回环抑制与压缩往返的完整单元测试。
- `crates/ios-cli/src/cmd/mod.rs` & `crates/ios-cli/src/main.rs`:
  - 注册 `viewer` 模块并在 CLI 根分发路由中挂载 `Viewer(cmd::viewer::ViewerCmd)`。

## 3. 验证命令与结果

### 3.1 单元测试套件验证
```powershell
cargo test -p ios-cli --all-features --locked cmd::viewer::tests::
```
结果：
- `cmd::viewer::tests::extended_clipboard_compression_round_trip` ... ok
- `cmd::viewer::tests::clipboard_bounds_and_loop_suppression` ... ok
- `cmd::viewer::tests::vnc_server_handshake_and_pointer_cleanup` ... ok
- `cmd::viewer::tests::vnc_server_hotkeys_and_buttons_mapping` ... ok
- `cmd::viewer::tests::web_viewer_auth_rejection` ... ok
- `cmd::viewer::tests::web_viewer_index_and_stream_lifecycle` ... ok
- `cmd::viewer::tests::web_viewer_input_endpoints_dispatch_actions` ... ok
- 7 passed, 0 failed.

## 4. 交付与待外部运行条件说明

- F05 全部三项子任务（F05-1、F05-2、F05-3）核心代码与测试均已交付并验证通过。
- 物理真机上实际画面投射、VNC 客户端互联及跨设备剪贴板真机端操作保留为 `NOT_RUN`，等待外部真机环境具备时执行。
