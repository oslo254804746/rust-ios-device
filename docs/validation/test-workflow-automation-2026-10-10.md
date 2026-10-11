> 本页保留 af20f6a 交付时的历史声明。2026-10-10 独立审查已否定其完整开发/验收结论；不能据本文将占位流程或 NOT_RUN 记为通过。当前代码与实际结果以 [修复记录](review-fixes-2026-10-10.md) 为准。

## 2026-10-11 续修实现

主代理已执行最终实际CLI/TLS-usbmux/ZIP/DTX/syslog的12项应用/XCTest测试及各故障子例，全部通过；完整adapter suite为29执行通过、1个POSIX skip。启动owner移出可取消work、DTX原错误及有界shutdown/drain已验证。[本轮证据与未完成范围](continuation-2026-10-11.md)优先于下方编辑时NOT_RUN声明；crash收集及完整现代native/platform范围继续。

每次初始执行和 failed-selector 重跑各保存一份完整 summary，最终 merged summary 仅表示当前结果。JSON finished 输出的 `attempts` 对象以及 `JUNIT_PATH.attempts.json` 原子 sidecar 保存 number、selectors、state、summary 和 error_class；state 区分 starting、completed、failed_tests、interrupted。首次失败、失败消息和中断时实际收集的部分事件不被最后一次通过覆盖。每次调用最多 10 次重跑，历史累计序列化结果和产物分别有 32 MiB 上限。

重跑 startup/result 错误现在返回原原因；不再吞掉失败后只打印先前 summary。重跑必须完整返回全部 selected failed cases，重复、遗漏、无关和未完成结果在改变 primary 前拒绝。安装仍在重跑循环外进行，缺前置条件不会重装有副作用包。

ActiveTestPlan 的 result recorder 属于原 session owner；等待失败或取消保留已处理事件。所有阶段退出均再执行统一 diagnostic/JUnit 写入、原 sessions terminate、log 原 task join 与本轮 owned bundle 清理。收尾错误也写入终态 sidecar，不能把报告成功覆盖为整个 workflow 成功。报告写入失败保留 primary/secondary 原对象。

调用处和 stage 包装把大 service/startup futures 放入 Box，缩小 Windows debug first-poll 状态。未来规模 gate 是局部门禁，真实 CLI + TLSmux/ZIP conduit/DTX/syslog peer 的完整路径与故障阶段由 regression fixture 独立验证；没有把 helper 单测当完整工作流验收。

本次新增结构化重跑历史与替换产物、严格 merge 拒绝、真实 InstallationProxy reverse-owned cleanup、原 Syslog NUL parser/EOF/partial/footer fixtures。此子代理禁止 Rust 构建，未声称它们 PASS。完整代表性设备合成 runner 仍 NOT_RUN；平台取消和各 startup 阶段覆盖依主代理最终实际结果登记。

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
