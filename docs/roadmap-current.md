# 当前工程调度

更新：2026-10-08。任务定义与最终验收以
[原工程路线图](engineering-roadmap-2026-09-30.md) 为准；本页用于承接当前工作，
不替代或缩减原任务。Q04 及全部子项已排除，其他原验收条件继续有效。

| 工作项 | 当前状态 | 交付边界和证据 |
| --- | --- | --- |
| L01/L02 完整扫描控制 | 主机子项已验收 | 原提交 `5e521e5`；[扫描控制](validation/tunnel-scan-control-2026-10-08.md) |
| L01/L02 成功连接后的退避清理 | 主机子项已验收 | `6bceff7`；窗口公平性、版本保护、期限回滚及原 native owner 收尾；[证据](validation/tunnel-success-bookkeeping-2026-10-08.md) |
| L01/L02 手动 selector 与 gate key 文本上限 | 实现待集成验收 | 独立 worktree 已交付；保留既有 4KiB 单字段及 4096 gate 数量上限，验证拒绝前无 Source/协调状态副作用 |
| F01 显式设备 doctor | 开发中 | 独立 worktree；既有信任的只读 session、Developer Mode 和 DDI signature 观察，保留无目标时 host schema1；缺元数据与显式 false 分开 |
| L03 重连 | 下一批待开发 | 可配置退避/预算/事件及 Darwin pasteboard watch；每次恢复重新发现服务，保留基线抑制与去重，只重建只读监听 |
| A03 常用 SDK | 待开发 | 设备/应用/有界文件读取/日志句柄，Python context/async 和 C ownership/error 合同集中交付 |

当前外部缺口：没有已指定的授权设备、iOS/USB/Wi-Fi 状态及对应 workflow fixture，
回归入口的 22 项真机行仍为 NOT_RUN；缺 macOS、Windows/UNC 和其他发布目标的
实际运行证据。恢复执行还缺明确机型、固件、模式转换和真实协议证据；不能用
现有 codec 或离线测试替代恢复循环。RSD/remoted 共存、实际 DDI 可用性、媒体/HID
和富内容剪贴板按原任务分别待条件。上述条件不阻止其他主机工作继续。

主 agent 统一集成、冻结被测源码、串行编译和批量验收。子 agent 在独立 worktree
开发或只读审查，禁止启动 Rust 编译。验收通过后本地提交；不推送、发布或部署。
