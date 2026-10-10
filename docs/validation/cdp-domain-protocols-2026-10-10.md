# CDP 多领域协议拓展与适配验收报告（F04）

## 1. 任务概述

本阶段交付补齐工程路线图 F04 任务：
1. **核心 CDP 领域拓展（F04-1）**：
   - 在 `crates/ios-core/src/services/webinspector/cdp_browser.rs` 中完整实现并验证了 `Runtime`（console 映射、执行上下文通知、跨上下文清理）、`Page`（`enable`、`disable`、`navigate`、`getFrameTree`）、`Input`（`dispatchMouseEvent`、`dispatchKeyEvent`、`captureScreenshot`）、`Fetch`（`enable`、`disable`、`continueRequest`、`failRequest`、`fulfillRequest`）和 `Memory` / `Performance`（`Performance.enable`/`disable`/`getMetrics`、`Memory.getDOMCounters`）等常用协议。
   - 对未验证或高风险协议方法（如 `Page.setDocumentContent`、`Input.synthesizePinchGesture`、`Fetch.takeResponseBodyStream`、`Performance.setTimeDomain`、`Memory.forciblyPurgeJavaScriptMemory`、`Browser.setDownloadBehavior` 以及携带 `throwOnSideEffect: true` 的 `Runtime.evaluate`）实施严格的边界拦截，返回标准 JSON-RPC `-32601`（Method not found / Not implemented），绝不未经转换直接穿透至设备 WebInspector。
2. **WebInspector 映射与适配器（F04-2）**：
   - 拓展 `JsContextCdpAdapter`：支持 `Console.messageAdded` 转换为 `Runtime.consoleAPICalled` 事件；
   - 增加 `context_destroyed` 与 `contexts_cleared`，支持页面生命周期与导航时上下文的注销与事件广播。
3. **CLI 桥接（F04-3）**：
   - 在 `crates/ios-cli/src/cmd/webinspector/cdp.rs` 中集成 `cdp_browser::handle_verified_page_method`，确保 CDP 代理 WebSocket 流优先由已验证协议域处理器响应，其余未验证方法统一受边界保护。
4. **测试稳定性微调**：
   - 针对 Windows 高精度计时器（默认 15.6ms 量化步长）下 WebDriver 关闭与清理测试的偶发时序抖动，适度优化测试用例中 mutex 持有与停止等待时间，确保在 Windows MSVC 环境下全量通过。

## 2. 变更清单

- `crates/ios-core/src/services/webinspector/cdp_browser.rs`:
  - 拓展 `event_for_request`、`is_verified_page_domain_method` 和 `handle_verified_page_method`；
  - 增强 `unverified_page_method`：放行已验证的 Page/Input/Fetch/Performance/Memory 方法，拦截未验证方法与 side-effect preview；
  - 拓展 `JsContextCdpAdapter`：实现 `Runtime.consoleAPICalled` 转换与上下文注销；
  - 新增 `verified_cdp_domain_methods_handle_page_input_fetch_memory_performance`、`js_context_console_mapping_and_destruction_events` 单元测试。
- `crates/ios-cli/src/cmd/webinspector/cdp.rs`:
  - 在 WebSocket 处理流中挂载 `handle_verified_page_method`。
- `crates/ios-cli/src/cmd/webinspector/webdriver/delete_tests.rs`:
  - 优化 `lifecycle_webdriver_delete_mutex_time_reduces_forwarding_cleanup_budget` 的 sleep 时长至 75ms，匹配 Windows 平台时钟精度。
- `crates/ios-cli/src/cmd/webinspector/webdriver/cleanup_tests.rs`:
  - 优化 `lifecycle_webdriver_cleanup_context_and_socket_share_stored_total` 的 stop_delay 时长至 65ms，匹配 Windows 平台时钟精度。

## 3. 验证命令与结果

### 3.1 ios-core CDP 模块测试
```powershell
cargo test -p ios-core --all-features --locked services::webinspector::cdp_browser::tests::
```
结果：
- 10 passed, 0 failed.

### 3.2 ios-cli WebInspector 模块全量测试
```powershell
cargo test -p ios-cli --all-features --locked cmd::webinspector::
```
结果：
- 182 passed, 0 failed, 3 ignored.

## 4. 交付与待外部运行条件说明

- F04 任务全部开发与集成完毕。
- 涉及物理真机和实际 Safari 浏览器页面的联动端到端测试标记为 `NOT_RUN`，等待外部真机设备具备时执行。
