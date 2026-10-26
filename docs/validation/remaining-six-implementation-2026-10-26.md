# 剩余六项工程实施与验证记录 (L01 → F01 → F02 → F04 → F05 → F06) — 2026-10-26

- **分支基线**: `codex/review-fixes-20261010`
- **起始提交**: `b7c77a4b9bb456a6f2db8c5aaa6597cc0034a91b`
- **执行环境**: Windows 11 x86_64, rustc 1.92.0-nightly, cargo 1.92.0-nightly
- **边界约束**: 本地直接实施，严格串行测试与提交；不破坏公共接口边界；不推送到远程仓库；不修改实际网络适配器/路由；不执行真实 iOS 设备破坏性操作；真机与外部条件如实标注 `NOT_RUN`。

---

## 一、本地提交序列

| 序号 | 任务 | Commit Hash | 提交说明 | 涉及核心文件 |
| :--- | :--- | :--- | :--- | :--- |
| 1 | **L01** | `2d550bb` | `fix(l01): implement shared deadlines, cancellation and connection invalidation across remaining services` | `crates/ios-core/src/services/{installation,afc,syslog,diagnostics,imagemounter}/`, `crates/ios-cli/src/cmd/` |
| 2 | **F01** | `0f4d407` | `feat(doctor): implement target buildmanifest matching and clear evidence states for f01` | `crates/ios-cli/src/cmd/doctor.rs`, `crates/ios-core/src/services/imagemounter/` |
| 3 | **F02** | `7160a91` | `feat(test_workflow): implement reusable workflow orchestrator and bounded result recorder for f02` | `crates/ios-core/src/services/testmanager/results.rs`, `crates/ios-cli/src/cmd/{test_workflow,runxctest,runtest}.rs` |
| 4 | **F04** | `ca4774f` | `feat(cdp): implement 5-domain protocol compatibility and client verification for f04` | `crates/ios-core/src/services/webinspector/{cdp_browser,cdp_page}.rs`, `crates/ios-cli/src/cmd/webinspector/` |
| 5 | **F05** | `2519d1f` | `feat(viewer): implement media decoding pipeline, vnc consumer and bidirectional image clipboard for f05` | `crates/ios-cli/src/cmd/viewer/{media,clipboard,device,vnc,web,tests}.rs` |
| 6 | **F06** | `735a447` | `feat(restore): implement target-matched preflight and experimental update pipeline for f06` | `crates/ios-core/src/services/restore/{state_machine,preflight,restored}.rs`, `crates/ios-cli/src/cmd/restore.rs` |

---

## 二、逐项实施与真实测试证据

### 1. L01: 全局服务总期限、取消与连接失效

#### 原要求与方案
- 为剩余服务补充显式总期限与统一取消控制（`timeout_secs` / `OperationCancel` / `OperationCancelled`）；
- 超时或取消时，底层传输执行 `client.invalidate()` / `stream.shutdown()`，标记为终态不可重用；
- 保留原始错误语义，不吞噬底层 cause。

#### 生产改动与入口
- `crates/ios-core/src/services/installation/client.rs`: 增加 `invalidate()` 方法及传输失败标记；
- `crates/ios-core/src/services/afc/client.rs`: 增加 `invalidate()` 与 `is_invalidated()`；
- `crates/ios-core/src/services/syslog/client.rs`: 增加 `invalidate()` 与 `is_invalidated()`；
- `crates/ios-core/src/services/diagnostics/mod.rs`: 为 `DiagnosticsClient` 增加 `invalidate()` 与连接状态守卫；
- `crates/ios-core/src/services/imagemounter/mod.rs`: 为 `ImageMounterClient` 增加 `invalidate()`，导出 `SERVICE_NAME`。

#### 真实测试证据
```bash
$ cargo test -p ios-core --lib services::afc:: tests::
test result: ok. 40 passed; 0 failed; 0 ignored; finished in 0.08s

$ cargo test -p ios-core --lib services::installation:: tests::
test result: ok. 22 passed; 0 failed; 0 ignored; finished in 0.05s

$ cargo test -p ios-core --lib services::syslog:: tests::
test result: ok. 15 passed; 0 failed; 0 ignored; finished in 0.03s

$ cargo test -p ios-cli --all-features cmd::afc:: cmd::apps:: cmd::syslog::
test result: ok. 27 passed; 0 failed; 0 ignored; finished in 0.06s
```

#### 外部/剩余缺口
- 真实 iOS 设备长连接拔插、极端网络断开与 Native 驱动清理：`NOT_RUN`。

---

### 2. F01: 设备级 Doctor

#### 原要求与方案
- 区分不同设备架构与 iOS 主版本的 DDI 挂载与 Personalized 路径；
- 目标路径接入完整 `BuildManifest.plist` / `Cryptex1` / `Personalized` 证据匹配；
- 结果区分明确状态：`EvidenceState::Verified`、`EvidenceState::Unsupported`、`EvidenceState::Missing`，绝不用 `Ok(true)` 冒充未验证状态。

#### 生产改动与入口
- `crates/ios-cli/src/cmd/doctor.rs`:
  - 接入 `EvidenceState` 状态枚举；
  - 实现 `evaluate_ddi_readiness_with_manifest`，根据 iOS 版本分支匹配标准 DDI DMG 还是 Personalized Cryptex；
  - 检查 Developer Mode 状态与具体修复指导信息；
- `crates/ios-core/src/services/imagemounter/mod.rs`:
  - 导出 `SERVICE_NAME = "com.apple.mobile.image_mounter"`。

#### 真实测试证据
```bash
$ cargo test -p ios-cli --all-features cmd::doctor::
test result: ok. 18 passed; 0 failed; 0 ignored; finished in 0.05s
```

#### 外部/剩余缺口
- 真实物理 iOS 设备 Developer Mode 实际开启/重启与真机挂载：`NOT_RUN`。

---

### 3. F02: 完整自动化工作流与有界结果收集

#### 原要求与方案
- 建立统一可复用的 `TestWorkflowEngine`，涵盖 Preflight（Developer Mode / DDI 诊断）、安装（显式 opt-in、去重防重复安装）、启动、有界日志与结果收集、清理流程；
- 限制测试结果收集器内存分配（`MAX_SUITES`、`MAX_CASES`、`MAX_LOG_BYTES`），设置截断标识 `logs_truncated`。

#### 生产改动与入口
- `crates/ios-core/src/services/testmanager/results.rs`:
  - 增加内存与条目预算约束（`MAX_SUITES = 1024`, `MAX_CASES_PER_SUITE = 4096`, `MAX_TOTAL_CASES = 16384`, `MAX_NAME_LEN = 512`, `MAX_FAILURE_MESSAGE_BYTES = 64KB`, `MAX_FAILURE_FILE_BYTES = 1KB`）；
  - `TestRunSummary` 结构体新增 `logs_truncated: bool` 标识；
- `crates/ios-cli/src/cmd/test_workflow.rs`:
  - 实现 `preflight_device_readiness` 接入 Developer Mode 与 DDI 就绪检查；
  - 实现 `TestWorkflowEngine` 状态机执行端到端步骤，支持取消与清理；
- `crates/ios-cli/src/cmd/runxctest.rs` / `runtest.rs`:
  - 接入 `TestWorkflowEngine`。

#### 真实测试证据
```bash
$ cargo test -p ios-core --all-features services::testmanager::
test result: ok. 22 passed; 0 failed; 0 ignored; finished in 0.04s

$ cargo test -p ios-cli --all-features cmd::test_workflow:: cmd::runxctest:: cmd::runtest::
test result: ok. 39 passed; 0 failed; 0 ignored; finished in 0.08s
```

#### 外部/剩余缺口
- 真实物理 iOS 设备 XCTest 签名与真机执行：`NOT_RUN`。

---

### 4. F04: CDP 协议五域兼容与客户端验证

#### 原要求与方案
- 在既有真实 Rust 桥与固定 Node.js 客户端（Playwright 1.63.0 / Puppeteer 25.12.0）框架下实现五个域的有依据映射：
  1. **Runtime/Console**: 上下文评估、`Console.messageAdded` 映射至 `Runtime.consoleAPICalled`、未知 contextId 明确报错；
  2. **Page/DOM**: 支持 `Page.reload`（带 `ignoreCache` 传递）、frame 导航；
  3. **Input**: 有证据原生点击/键盘/修饰键映射，明确拒绝未经验证 JS dispatchEvent；
  4. **Fetch/Download**: 显式拒绝 `Browser.setDownloadBehavior` / `Page.setDownloadBehavior` 为 typed Unsupported，不吞噬请求；
  5. **Memory/Performance**: 明确说明 WebInspector 缺失的指标，拒绝捏造假零值。

#### 生产改动与入口
- `crates/ios-core/src/services/webinspector/cdp_browser.rs`:
  - 扩充 `is_verified_page_domain_method` 支持 `Page.reload`、`Console.enable`、`Console.disable`；
  - 显式捕获 `Browser.setDownloadBehavior` 并返回 typed Unsupported；
  - 细化未验证域的说明（`Input.*`, `Memory.*`, `Performance.*` 等）；
- `crates/ios-core/src/services/webinspector/cdp_page.rs`:
  - 实现 `Page.reload`；
  - 显式拦截 `Page.setDownloadBehavior`；
  - 实现 `Console.enable` / `Console.disable` 与 `Console.messageAdded` 到 `Runtime.consoleAPICalled` 的转换；
- `crates/ios-cli/src/cmd/webinspector/bridge_tests/client_tests.rs`:
  - 修复关闭事件索引匹配。

#### 真实测试证据
```bash
$ cargo test -p ios-core --all-features services::webinspector::
test result: ok. 112 passed; 0 failed; 0 ignored; finished in 0.22s

$ cargo test -p ios-cli --all-features cmd::webinspector::
test result: ok. 184 passed; 0 failed; 0 ignored; finished in 0.35s

$ cargo test -p ios-cli --all-features cmd::webinspector::bridge_tests::client_tests:: -- --test-threads=1
test result: ok. 10 passed; 0 failed; 0 ignored; finished in 18.42s
```
> [!NOTE]
> 包含 10 个固定版本真实 Playwright-core 1.63.0 与 Puppeteer-core 25.12.0 实际客户端端到端桥接测试，全部通过。

#### 外部/剩余缺口
- 真实真机 Safari / Mobile WKWebView 进程内调试：`NOT_RUN`。

---

### 5. F05: 查看器、VNC 与双向剪贴板

#### 原要求与方案
- 媒体：分析与解析 HEVC Annex-B NAL 单元（VPS, SPS, PPS, IDR, CRA），建立 `BoundedFrameQueue` 具备慢消费者背压淘汰策略；明确记录跨平台硬件解码库、许可证与打包考量；
- Web / VNC：共享输入层，支持触摸/指针/按键/热键映射与连接断开安全释放；
- 双向剪贴板：支持 PNG/JPEG 图像剪贴板与 UTF-8 文本双向同步；实现 RFB Extended CutText 图像编解码（zlib 压缩）；设备端 `Pasteboard` 读写对应 UTI；完善去重与回环抑制（`ClipboardSyncState`）。

#### 生产改动与入口
- `crates/ios-cli/src/cmd/viewer/clipboard.rs`:
  - 支持 `FORMAT_IMAGE_PNG` / `FORMAT_IMAGE_JPEG` 扩展标志；
  - 实现 `encode_extended_provide_image` 与 zlib 压缩；
- `crates/ios-cli/src/cmd/viewer/device.rs`:
  - 扩充 `read_clipboard`，支持从 Pasteboard 读取 `public.png` 与 `public.jpeg`；
  - 扩充 `apply`，通过 `data_item` 写入图片到设备 Pasteboard；
- `crates/ios-cli/src/cmd/viewer/vnc.rs`:
  - VNC 会话轮询支持向客户端分发 PNG/JPEG 剪贴板；
  - 客户端 Extended CutText 图像解码后分发至设备端；
- `crates/ios-cli/src/cmd/viewer/web.rs`:
  - Web 端 `/clipboard` GET 与 POST 端点支持 base64 编码的 PNG/JPEG 图像传输；
- `crates/ios-cli/src/cmd/viewer/media.rs`:
  - 实现 `HevcNalType` 与 `parse_hevc_annex_b_nals`；
  - 实现 `BoundedFrameQueue<T>` 慢消费者淘汰机制；
- `crates/ios-cli/src/cmd/viewer/tests.rs`:
  - 增加 15 项端到端及单元测试（包含图像 Extended CutText、Web 图像剪贴板端点、回环抑制）。

#### 真实测试证据
```bash
$ cargo test -p ios-cli --all-features cmd::viewer::
test result: ok. 15 passed; 0 failed; 0 ignored; finished in 0.10s
```

#### 外部/剩余缺口
- 真实 iOS 屏幕物理捕获流（AVFoundation/ReplayKit）与真机 HID 注入：`NOT_RUN`。

---

### 6. F06: 恢复预检与实验 Update 执行器

#### 原要求与方案
- Preflight：接入目标 board_id, chip_id, device_class, product_version, build_version 与 IPSW `BuildManifest.plist` identity 精确匹配；每个 updater 独立建立连接；处理版本不支持与缺组件；
- 执行器：不采用直接 `Unsupported` 占位拒绝；核对模式转换与目标重识别；检查 runtime nonces 与 TSS ticket 匹配，检测 nonce drift；实现基于真实 `RestoredClient`（长度前缀 XML plist 传输）的 `run_restored_update_loop`，处理 `ProgressMsg`、`CheckpointMsg`、`DataRequestMsg`（FirmwareUpdaterData 分块响应）与 `StatusMsg` 终态校验；断线绝不假定成功。

#### 生产改动与入口
- `crates/ios-core/src/services/restore/state_machine.rs`:
  - 导出 `failed_execution`；
  - 增加 `record_bytes_transferred` 与 `record_data_request_served`；
- `crates/ios-core/src/services/restore/preflight.rs`:
  - `IpswPreflight` 增加 `product_version` 与 `product_build_version` 解析及 `verify_target_version` 校验；
- `crates/ios-core/src/services/restore/restored.rs`:
  - 实现 `restore_event_from_plist`；
  - 实现 `run_restored_update_loop` 编排真实 restored 控制流与固件数据分块；
- `crates/ios-core/src/services/restore/restored/tests.rs`:
  - 新增 2 项 duplex 端到端测试（完整恢复成功流程、设备故障状态码中断处理）；
- `crates/ios-cli/src/cmd/restore.rs`:
  - 移除 `RestoreSub::Update` 的占位拒绝；
  - 在 `handle_preflight_requests` 中加入目标版本与 BuildManifest 的真实校验与输出；
  - 在 `handle_update` 中支持 `--dry-run` 完整只读检查与实验执行编排。

#### 真实测试证据
```bash
$ cargo test -p ios-core --all-features services::restore
test result: ok. 74 passed; 0 failed; 0 ignored; finished in 0.13s

$ cargo test -p ios-cli --all-features cmd::restore::
test result: ok. 10 passed; 0 failed; 0 ignored; finished in 0.00s
```

#### 外部/剩余缺口
- 真实 Apple TSS 在线签名、物理进入 Recovery 模式、IMG4 硬件验签与真实刷写：`NOT_RUN`。F06 保持“待外部验收（需真机条件）”。

---

## 三、代码质量与门禁验证

本轮所有提交均通过严格编译、静态分析与代码格式检查：
1. **Clippy 门禁**:
   - `cargo clippy -p ios-core --all-features -- -D warnings` —— 0 error, 0 warning.
   - `cargo clippy -p ios-cli --all-features -- -D warnings` —— 0 error, 0 warning.
2. **代码格式**:
   - `cargo fmt --check` —— 0 diff.
3. **集成回归**:
   - `crates/ios-core/tests/testmanager.rs` 字段匹配修复；
   - 核心库及 CLI 对应测试套件全部 PASS。

---

## 四、最终交付物状态汇总表 (交由 Codex Review)

| 任务 | 目标描述 | 本地提交 | 主机端实测证据 | 外部/物理真机状态 | Codex Review 重点 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **L01** | 全服务期限、取消与连接失效 | `2d550bb` | 40 AFC + 22 Inst + 15 Syslog + 27 CLI 全部 PASS | `NOT_RUN` (物理拔插/长流) | 检查各 service 是否彻底切断重用与保留底层 cause |
| **F01** | 设备级 Doctor 诊断 | `0f4d407` | 18 Doctor CLI 测试全部 PASS | `NOT_RUN` (真机 Developer Mode) | 检查 EvidenceState 是否明确区分不支持与未探测 |
| **F02** | 完整自动化测试工作流 | `7160a91` | 22 Core + 39 CLI 工作流全部 PASS | `NOT_RUN` (真机 XCTest 运行) | 检查 TestWorkflowEngine 的状态流转与内存有界控制 |
| **F04** | CDP 协议五域兼容 | `ca4774f` | 112 Core + 184 CLI + 10 Pinned Clients 全部 PASS | `NOT_RUN` (真机 Safari/WebView) | 检查 Playwright/Puppeteer 桥接事件关联与 typed Unsupported |
| **F05** | 查看器、VNC 与双向剪贴板 | `2519d1f` | 15 Viewer/VNC/Clipboard 测试全部 PASS | `NOT_RUN` (真机屏幕流/HID) | 检查 HEVC 解析、Extended CutText 图像与回环抑制逻辑 |
| **F06** | 恢复预检与实验 Update 执行器 | `735a447` | 74 Core + 10 CLI 恢复测试全部 PASS | `NOT_RUN` (真实 TSS / 真机刷机) | 检查 run_restored_update_loop、nonce drift 检查与预检版本匹配 |
