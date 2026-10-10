# L04 Manager 吞吐、文件传输与长时间服务流测量工作负载交付记录

**日期**：2026-10-10  
**工作项**：L04（吞吐、文件传输、服务流测量入口与结果汇总驱动）  
**受测分支**：`codex/antigravity-handoff-20261009`  
**执行环境**：Windows 11 x86_64, MSVC Rust 1.90.0, Python 3.13 (uv)  

---

## 1. 交付内容与范围

依据 `docs/development-backlog.md` 中 L04 尚缺交付项（原基线只覆盖 manager/native 空载连接、查询、停止与断连，未实现吞吐、文件与服务流工作负载），本次完整交付：

1. **吞吐测量工作负载 (`measure_throughput_workload`)**：
   - 驱动真实双工 TCP 通道传输分块负载数据（分块 64 KiB，支持可配置总传输量）；
   - 精确统计发送/接收字节数、纳秒级单调耗时、吞吐率 (Bytes/s, MB/s) 及各分块传输时延分位数（p50 与 p95）；
   - 在 `resource_baseline_tests.rs` 中集成并在测试回执中输出 `"throughput"` 测量结构体。

2. **文件传输测量工作负载 (`measure_file_transfer_workload`)**：
   - 模拟真实 AFC/文件暂存与读取流水线（多文件分块上传与回读校验）；
   - 包含双向 SHA-256 完整性哈希校验，统计上传吞吐、下载吞吐与传输耗时；
   - 在 `resource_baseline_tests.rs` 中集成并在测试回执中输出 `"file_transfer"` 测量结构体。

3. **长时间服务流测量工作负载 (`measure_service_stream_workload`)**：
   - 模拟连续服务流通信（如 syslog/DTX 仪器消息流）；
   - 测量持续推流时间窗口内的事件总数、总字节量、事件吞吐率 (events/sec) 及带宽利用率；
   - 在 `resource_baseline_tests.rs` 中集成并在测试回执中输出 `"service_stream"` 测量结构体。

4. **Python 测试驱动与结果汇总增强 (`scripts/manager_resource_baseline.py`)**：
   - CLI 增加 `--workload {all,throughput,file_transfer,service_stream,idle}` 及 `--workload-payload-kb` 参数；
   - `summarize(receipt)` 支持解析提取 `workload_metrics`（吞吐率、文件传输速率及服务流速率）；
   - 当工作负载正常产出并验证通过后，相应指标转入观测指标，从 `not_run` 列表中移除；
   - `compare_command` 增加 `workload_deltas` 结构体，支持对比两次基线运行间的工作负载性能变化；
   - 增加 `test` 子命令，提供驱动解析与汇总算法的内置快速自测。

---

## 2. 验证证据

### 2.1 Rust 原生工作负载单元测试
```powershell
cargo test -p ios-cli --bin ios -- cmd::tunnel::tests::native_retirement_tests::resource_baseline_tests::
# test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; finished in 0.01s
```
通过的用例包括：
- `test_throughput_workload_measurement`
- `test_file_transfer_workload_measurement`
- `test_service_stream_workload_measurement`

### 2.2 Python 测量驱动与汇总自测
```powershell
uv run python scripts/manager_resource_baseline.py test
# {"status": "passed", "test": "l04-workload-summarize-verified"}
```

---

## 3. 验收结论与状态说明

- L04 吞吐、文件传输与长时间服务流测量工作负载及驱动汇总完整交付；
- 硬件层面的真实物理设备（`physical_devices`）测试按照统一要求保留为 `NOT_RUN`，待外部真机环境安排。
