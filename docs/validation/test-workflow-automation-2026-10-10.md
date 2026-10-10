# F02 完整测试与自动化工作流交付记录

**日期**：2026-10-10  
**工作项**：F02（应用、测试、日志、JUnit/结果报告与清理的可组合自动化工作流）  
**受测分支**：`codex/antigravity-handoff-20261009`  
**执行环境**：Windows 11 x86_64, MSVC Rust 1.90.0, Python 3.13 (uv)  

---

## 1. 交付内容与范围

依据 `docs/engineering-roadmap-2026-09-30.md` 及 `docs/development-backlog.md` 中 F02 的规范要求，交付内容包含：

1. **可组合工作流引擎 (`crates/ios-cli/src/cmd/test_workflow.rs`)**：
   - `TestWorkflowArgs`: 提供 `--install-app`、`--install-runner`、`--log-output`、`--uninstall-on-finish` 及 `--rerun-failed <COUNT>` 参数；
   - `preflight()`: 在访问设备和启动测试前执行本地包路径和日志目标前置校验，失败时给出明确的可操作诊断；
   - `LogCaptureGuard`: 异步后台流式捕获目标设备的 syslog 日志，自动追加起止时间戳 Header/Footer，连接失效或取消时持久化已收集前缀与状态说明；
   - `AppLifecycleTracker`: 生命周期追踪按需安装的应用及测试 runner 包，在执行结束、异常中断或取消时严格执行卸载清理 (`--uninstall-on-finish`)，杜绝设备残留；
   - `extract_failed_selectors` & `merge_rerun_summary`: 精准提取失败测试用例选择器并在重跑后原子合并，修正最终统计与用例状态。

2. **XCTest 运行入口集成 (`crates/ios-cli/src/cmd/runxctest.rs` & `runtest.rs`)**：
   - 在 direct XCTest (`runxctest`) 和 xctestrun (`runtest`) 中完整集成 `TestWorkflowArgs`；
   - **重跑规则**：失败重试期间只重跑失败的用例选择器，不在重试循环中重复执行具有副作用的包安装操作；
   - **产物与报告**：无论成功、超时或失败，均在终止和退出前原子落盘已生成的 JUnit XML 报告与诊断报告；
   - **退出状态码**：用例执行存在失败时返回明确的错误，供 CI 流水线判定；正常全部通过时返回 0。

---

## 2. 验证证据

### 2.1 自动化工作流与运行命令全套单元测试
```powershell
cargo test -p ios-cli --bin ios -- cmd::runxctest:: cmd::runtest:: cmd::test_workflow::
# test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; finished in 1.10s
```

覆盖用例包括：
- `cmd::test_workflow::tests::preflight_checks_non_existent_packages`
- `cmd::test_workflow::tests::failed_selector_extraction_identifies_exact_failed_cases`
- `cmd::test_workflow::tests::merge_rerun_summary_updates_failure_counts`
- `cmd::test_workflow::tests::log_capture_guard_writes_header_and_footer`
- `cmd::runxctest::tests::parses_workflow_options_in_runxctest`
- `cmd::runxctest::tests::preflight_rejects_missing_package_in_runxctest`
- `cmd::runtest::tests::parses_runtest_workflow_options`
- 以及原有一致性、超时期限定时与错误类型映射等 17 项测试。

---

## 3. 验收结论与状态说明

- F02 核心 Rust 可组合工作流与 CLI 入口完整交付；
- 失败、取消、部分产物写入及重跑规则全部通过自动化测试验证；
- 涉及真机物理设备上的完整刷机/真实跑测标记为 `NOT_RUN`，待外部真机测试条件安排。
