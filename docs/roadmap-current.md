# 当前工程调度

更新：2026-10-08。任务定义与最终验收以
[原工程路线图](engineering-roadmap-2026-09-30.md) 为准；本页用于承接当前工作，
不替代或缩减原任务。Q04 及全部子项已排除，其他原验收条件继续有效。

| 工作项 | 当前状态 | 交付边界和证据 |
| --- | --- | --- |
| L01/L02 完整扫描控制 | 主机子项已验收 | 原提交 `5e521e5`；[扫描控制](validation/tunnel-scan-control-2026-10-08.md) |
| L01/L02 成功连接后的退避清理 | 主机子项已验收 | `6bceff7`；窗口公平性、版本保护、期限回滚及原 native owner 收尾；[证据](validation/tunnel-success-bookkeeping-2026-10-08.md) |
| L01/L02 手动 selector 与 gate key 文本上限 | 主机子项已验收 | `f0ec223`；既有 4KiB/4096 上限在 key/lock/Source 前生效，原 native owner 保留/收尾；[证据](validation/device-doctor-manual-admission-2026-10-08.md) |
| F01 显式设备 doctor | 主机子项已验收 | `71a59fc`；既有信任只读 session、Developer Mode/DDI signature，host schema1/target schema2，typed 类别与原 error；`82c6c56` 拒绝 Unix FIFO；[证据](validation/device-doctor-manual-admission-2026-10-08.md) |
| F01 既有 RSD route | 独立 worktree 开发中 | checked 只读观察、目标标识核对、完整目录与 missing/malformed 分开；不激活或接管 native tunnel，当前主区仍 not_probed |
| L03 manager 重连 | 独立 worktree 开发中 | serve 可配置退避/预算/状态、identity/epoch/version 与原 native owner 回收，待集中验收；manual 恢复、跨扫描停止与 Darwin watch 仍待交付 |
| A03 原生前置 | 静态交付待验收 | AFC caller byte limit；Browse checked 总期限、累计 wire/帧/条目预算和失效后复用拒绝；待主 agent 集成编译 |
| A03 Python/C 常用 SDK | 待开发 | 设备/应用/文件/日志句柄，Python context/async 和 C ownership/error/取消/并发合同与示例集中交付 |

当前外部缺口：没有已指定的授权设备、iOS/USB/Wi-Fi 状态及对应 workflow fixture，
回归入口的 22 项真机行仍为 NOT_RUN；缺 macOS、Windows/UNC 和其他发布目标的
实际运行证据。恢复执行还缺明确机型、固件、模式转换和真实协议证据；不能用
现有 codec 或离线测试替代恢复循环。RSD/remoted 共存、实际 DDI 可用性、媒体/HID
和富内容剪贴板按原任务分别待条件。doctor 的 OS metadata/regular file IO 不能物理取消，runtime 退出可能等待 blocking
worker；Unix FIFO 已拒绝，此限制仍需独立处理。上述条件不阻止其他主机工作继续。

主 agent 统一集成、冻结被测源码、串行编译和批量验收。子 agent 在独立 worktree
开发或只读审查，禁止启动 Rust 编译。验收通过后本地提交；不推送、发布或部署。
