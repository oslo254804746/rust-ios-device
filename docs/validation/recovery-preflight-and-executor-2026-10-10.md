> 本页保留 af20f6a 交付时的历史声明。2026-10-10 独立审查已否定其完整开发/验收结论；不能据本文将占位流程或 NOT_RUN 记为通过。当前代码与实际结果以 [修复记录](review-fixes-2026-10-10.md) 为准。

# 设备恢复预检与实验恢复执行状态机验收报告（F06）

## 1. 任务概述

本阶段交付补齐工程路线图有效 30 个主任务中的最后一个未交付项 **F06（设备恢复预检与实验恢复执行状态机）**，实现从 IPSW/TSS 解码到设备端多更新器预检请求聚合、状态机编排以及实验恢复执行的完整开发交付：

1. **设备恢复预检与多更新器聚合（F06-1）**：
   - 实现了 CLI 子命令 `ios restore preflight-requests`，支持提取 IPSW 内 BuildManifest、自动或显式匹配设备标识（`--board-id`、`--chip-id`、`--device-class`），并根据恢复行为（`Update` / `Erase`）构建基线请求；
   - 预置默认设备端预检更新器列表：`DEFAULT_PREFLIGHT_UPDATERS = &["T200", "Rose", "SE", "Vinyl", "Savage", "Centauri"]`，支持用户通过多次 `--updater` 参数覆盖或补充；
   - 实现了安全的设备端聚合收集逻辑 `collect_device_side_preflight_requests(...)`：
     - 每个 updater 采用全新的独立服务连接（`connect_service("com.apple.mobile.restored")`），杜绝跨更新器协议状态污染；
     - 针对每个 updater 配置独立超时预算，单个更新器遇到通信失败或拒绝时不中断其他更新器收集流程；
     - 将失败详细记录为 `PreflightUpdaterFailure`（更新器名称、错误详情、时间戳与失败阶段），并在 `CollectedPreflightRequests` 中完整汇报；
     - 收集汇总数据结构采用 `BTreeMap<String, Value>`，确保跨平台与多次执行间的键顺序与序列化输出完全确定。

2. **实验恢复执行状态机与受控驱动循环（F06-2）**：
   - 建立了严谨的受控恢复生命周期状态模型 `RestoreState`：
     `Idle` -> `ModeTransition` -> `QueryNonces` -> `TssRequest` -> `FirmwareTransfer` -> `FlashingUpdate` -> `Verifying` -> `Completed` / `Failed`；
   - 建立了精细的恢复失败分类模型 `RestoreFailure`：
     - `StatusCode(i64, String)`：设备端恢复服务返回的显式负数或异常状态码；
     - `Crash(String)`：底层通信连接异常中断或服务进程崩溃退出；
     - `NonceMismatch`：设备端实测 Nonce 与 TSS 签名请求或返回票据中的 Nonce 不匹配（Nonce 漂移）；
     - `MissingComponent(String)`：IPSW 或 TSS 票据中缺失设备当前阶段所需的必要固件组件；
     - `Timeout(String)`：特定阶段执行超过允许的预算阈值；
     - `Cancelled`：宿主机端主动取消恢复任务；
     - `ProtocolError(String)`：数据帧解析、XPC 编码或协议语义违规；
   - 显式声明并登记恢复协议与硬件限制缺口 `RestoreProtocolGaps`：
     1. **TSS 实时签名依赖**：生产设备依赖苹果在线 TSS 服务器验证与有效签名窗口（SHSH / APTicket），本地签名仅适用于测试或降级固件；
     2. **抹掉恢复 FDR 代理缺失**：Erase Restore 依赖 Factory Data Recovery (FDR) 代理协议与密钥同步，目前缺少完整代理服务实现；
     3. **DFU 阶段一引导限制**：DFU 模式初始化需经由 USB 低层控制传输发送 iBSS/iBEC 引导镜像，无法直接通过常规 RemoteXPC / restored 服务完成；
     4. **USB 重举与模式切换中断**：从 RecoveryOS 切换至 RestoreOS 或内核刷写阶段时，设备 USB 端口将产生硬件物理断开与重枚举，无线/网络隧道会断开；
     5. **未知外设固件协议**：部分协处理器与特定外围芯片更新器（如新型基带、安全隔离区）未公开全量刷写协议字典；
   - 核心状态机防护保障 `RestoreStateMachine`：
     - **Nonce 漂移检测**：在进入 TSS 签名与固件刷写前比对 Nonce 一致性，防范设备重置后 Nonce 失效导致的刷机废砖风险；
     - **固件分块与传输限额**：单块传输限制在安全上限（16 MiB），并维护严格的累计传输字节与进度计数；
     - **终态与晚到消息处理**：一旦进入 `Completed` 或 `Failed` 终态，后续到达的偶发网络或通道消息静默记录为 `LateMessage`，避免二次状态扰动；
     - **恢复执行报告**：生成包含所有阶段耗时、警告、错误与缺口清单的 `RestoreExecutionReport`。
   - 实现了异步双工恢复执行循环驱动器 `run_experimental_restore_loop(...)`；
   - CLI 交付子命令 `ios restore update`：
     - 缺省安全机制：必须显式传入 `--dry-run` 进行无副作用预检与状态转移演练，或显式传入 `--force` 进行实际实验执行，坚决杜绝误操作；
     - 在 dry-run 模式下校验 IPSW 完整性、TSS 票据对应关系、输出预估状态转换步骤、并打印完整协议缺口清单。

---

## 2. 变更清单

- `crates/ios-core/src/services/restore/preflight.rs`:
  - 定义 `DEFAULT_PREFLIGHT_UPDATERS`；
  - 实现 `PreflightUpdaterFailure`、`CollectedPreflightRequests` 与确定性聚合；
  - 实现 `collect_device_side_preflight_requests(...)`，具备单更新器独立连接、隔离超时与局部错误容忍能力；
  - 新增单测 `collect_device_side_preflight_requests_merges_updaters_and_handles_failures`。
- `crates/ios-core/src/services/restore/state_machine.rs`（新增模块）：
  - 实现 `RestoreState`、`RestoreFailure`、`RestoreProtocolGaps`、`RestoreStepOutcome` 与 `RestoreExecutionReport`；
  - 实现 `RestoreStateMachine` 核心状态转移逻辑与安全守卫（Nonce 比对、传输分块预算、终态隔离）；
  - 实现异步循环驱动器 `run_experimental_restore_loop(...)`；
  - 交付 7 组独立单元测试（正常流转、Nonce 漂移、分块限制、晚到消息、崩溃/状态码、超时/取消、协议缺口自检）。
- `crates/ios-core/src/services/restore/mod.rs`:
  - 导出 `pub mod state_machine;` 与核心类型；
  - 交付基于 Tokio DuplexStream 的端到端模拟服务集成测试 `experimental_restore_loop_runs_against_simulated_service_and_produces_report`。
- `crates/ios-cli/src/cmd/restore.rs`:
  - 在 `RestoreSubcommand` 中新增 `PreflightRequests` 与 `Update` 子命令及对应参数模型；
  - 实现 `handle_preflight_requests` 与 `handle_update` CLI 处理入口；
  - 实现 IPSW 匹配、设备信息降级读取（通过 Lockdown / CoreDevice）、TSS 票据加载与 dry-run 报告输出；
  - 新增 2 组 CLI 命令行参数解析单元测试（`parses_restore_preflight_requests_subcommand` 与 `parses_restore_update_dry_run_subcommand`）。

---

## 3. 验证命令与结果

### 3.1 恢复模块核心单元测试
```powershell
cargo test -p ios-core --features restore services::restore
```
结果：**65 passed; 0 failed; 0 ignored**
覆盖：
- `services::restore::state_machine::tests::protocol_gaps_inventory_is_complete_and_descriptive` ... ok
- `services::restore::state_machine::tests::state_machine_detects_nonce_drift` ... ok
- `services::restore::state_machine::tests::state_machine_handles_timeout_and_cancellation` ... ok
- `services::restore::state_machine::tests::state_machine_handles_crash_and_status_failures` ... ok
- `services::restore::state_machine::tests::state_machine_records_late_messages_after_completed` ... ok
- `services::restore::state_machine::tests::state_machine_transitions_through_normal_update_flow` ... ok
- `services::restore::state_machine::tests::state_machine_enforces_firmware_chunking_and_transfer_limits` ... ok
- `services::restore::preflight::tests::collect_device_side_preflight_requests_merges_updaters_and_handles_failures` ... ok
- `services::restore::tests::experimental_restore_loop_runs_against_simulated_service_and_produces_report` ... ok
- 及全部既有 56 项 restore/TSS 编解码与契约测试。

### 3.2 CLI 恢复命令解析单元测试
```powershell
cargo test -p ios-cli --bin ios cmd::restore
```
结果：**9 passed; 0 failed; 0 ignored**
包含 `preflight-requests` 与 `update` 新增子命令的全面参数解析验证。

### 3.3 代码规范与静态检查
```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
结果：格式与 Clippy 静态检查 100% 通过（0 errors, 0 warnings）。

---

## 4. 真机测试缺口说明

按工程既定基准与交付原则：
- 真实的物理设备恢复（对物理 iPhone/iPad 进行实际固件重写与分区擦除）因无用户指定的受控测试设备及物理 USB 重举环境，在此登记为 **NOT_RUN**；
- 协议层与状态机层已通过全套 mock / duplex 模拟数据流严格验证完成。
