# 隧道与管理器生命周期及元数据字节边界验收报告（L01 / L02）

## 1. 任务概述

本阶段交付补齐工程路线图 L01 与 L02 剩余的协议/服务生命周期与管理器规则：
1. **元数据字节边界完整性（L02）**：在 Manager HTTP 路由入口（`agent_get_tunnel`、`agent_delete_tunnel`、`agent_connect_authorized`）补齐针对 `udid` 的 `admission::selectors` 校验，严格限制字段在 4096 字节（4KiB）之内；超限请求直接以 `ResourceLimit` / 400 Bad Request 拒绝，防止未授权或畸形大字段穿透至内部协调器与数据表。
2. **生命周期规则与手动 Epoch 驱动（L01 / L02）**：
   - 暴露 `POST /refresh` 管理端点，驱动 `state.refresh_once()` 主动执行扫描刷新与设备元数据同步；
   - 暴露 `POST /epoch` 管理端点，驱动 `state.clear()` 执行手动 Epoch 切换，安全注销旧代 global runtime，清理未确认状态与失效重试，同时维持持久意图（Manual route / Stopped）；
   - 维持所有长时间流所有权、清理超时双重错误保留（Primary error + Cleanup error）与取消后状态复用隔离。
3. **并发跟踪与重连状态事件修复**：在 `reconnect_tests` 引入 `tracing::callsite::rebuild_interest_cache()`，修复跨测试并发时 Callsite Interest 缓存导致事件订阅未触发的问题，确保全部重连生命周期计数与状态转移断言稳定通过。

## 2. 变更清单

- `crates/ios-cli/src/cmd/tunnel.rs`:
  - `agent_connect_authorized`: 增加 `admission::selectors(&query.udid, None)` 检查。
  - `agent_get_tunnel`: 增加 `admission::selectors(&udid, None)` 检查。
  - `agent_delete_tunnel`: 增加 `admission::selectors(&udid, None)` 检查。
  - `agent_router`: 注册 `POST /refresh` 与 `POST /epoch` 路由。
  - 实现 `agent_refresh` 与 `agent_epoch` 处理函数，受 `Permission::ManageServer` 鉴权保护。
  - 新增单元测试 `agent_refresh_and_epoch_endpoints_execute` 与 `agent_endpoints_reject_oversized_selectors`。
- `crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/reconnect_tests.rs`:
  - 在 `subscriber()` 中调用 `tracing::callsite::rebuild_interest_cache()`，保证并行测试环境下的跟踪事件可靠捕获。

## 3. 验证命令与结果

### 3.1 新增端点与边界测试
```powershell
cargo test -p ios-cli --all-features --locked cmd::tunnel::tests::agent_
```
结果：
- `agent_endpoints_reject_oversized_selectors` ... ok
- `agent_refresh_and_epoch_endpoints_execute` ... ok
- 2 passed, 0 failed.

### 3.2 隧道模块全量测试
```powershell
cargo test -p ios-cli --all-features --locked cmd::tunnel::
```
结果：
- 335 passed, 0 failed, 2 ignored (L04 explicit baseline measured gates).

## 4. 交付与待外部运行条件说明

- L01 / L02 任务核心功能与元数据边界已全部交付并经验证。
- 物理真机拔插与真实 Wi-Fi 中断测试继续标记为 `NOT_RUN`，等待用户后续统一安排。
