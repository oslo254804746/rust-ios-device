# 当前工程调度

更新：2026-10-09。任务定义与最终验收以
[原工程路线图](engineering-roadmap-2026-09-30.md) 为准；本页用于承接当前工作，
不替代或缩减原任务。Q04 及全部子项已排除，其他原验收条件继续有效。

按2026-10-09用户修订的开发口径，有效30个主任务中，22个开发完成、
8个仍缺开发交付：L01、L02、L04、F01、F02、F04–F06。
只等待验证测试或外部运行条件的已交付实现计为完成；实际NOT_RUN仍保留。
具体尚缺的代码/工具交付见[待开发清单](development-backlog.md)。

| 工作项 | 当前状态 | 交付边界和证据 |
| --- | --- | --- |
| L01/L02 完整扫描控制 | 主机子项已验收 | 原提交 `5e521e5`；[扫描控制](validation/tunnel-scan-control-2026-10-08.md) |
| L01/L02 成功连接后的退避清理 | 主机子项已验收 | `6bceff7`；窗口公平性、版本保护、期限回滚及原 native owner 收尾；[证据](validation/tunnel-success-bookkeeping-2026-10-08.md) |
| L01/L02 手动 selector 与 gate key 文本上限 | 主机子项已验收 | `f0ec223`；既有 4KiB/4096 上限在 key/lock/Source 前生效，原 native owner 保留/收尾；[证据](validation/device-doctor-manual-admission-2026-10-08.md) |
| F01 显式设备 doctor | 主机子项已验收 | `71a59fc`；既有信任只读 session、Developer Mode/DDI signature，host schema1/target schema2，typed 类别与原 error；`82c6c56` 拒绝 Unix FIFO；[证据](validation/device-doctor-manual-admission-2026-10-08.md) |
| F01 既有 RSD route | 主机子项已验收 | `838c4ed`；显式 IPv6/已有 proxy 的总期限只读观察、目标核对、完整目录与 missing/malformed 分开；不激活/接管 tunnel 或执行 advertised 服务；[证据](validation/doctor-rsd-syslog-2026-10-09.md) |
| L03 manager 可配置 episode | 主机子项已验收 | `1a604af`；serve 退避/有限预算/准确状态、identity/epoch/version 与原 native owner 回收；[证据](validation/manager-reconnect-2026-10-08.md)。manual 恢复/跨扫描 stop 已另行验收；Darwin watch 继续 |
| A03 原生有界读取 | 主机子项已验收 | `655733c`；AFC caller byte limit；Browse checked 总期限、累计 wire/帧/条目预算和失效后复用拒绝；[证据与迁移](validation/bounded-service-read-2026-10-08.md) |
| A03 日志取消/失效 framing | 主机子项已验收 | `f077674`；取消保留已读前缀、EOF/IO/超长后 terminal 并拒绝复用；[证据](validation/doctor-rsd-syslog-2026-10-09.md) |
| A03 SDK session 前置 | 主机子项已验收 | `2b906d7`；共同 owner、单一绝对期限/取消/并发、checked existing-trust session 与 owned prepared-service 观察；[证据](validation/sdk-owned-session-2026-10-09.md)。Rust device query 已另行验收；binding 接线继续 |
| L03 Darwin剪贴板watch重连 | 主任务开发交付 | `3d90d12`；Darwin watch 错误类型分类解包、重连 episode 取消时机、事件断线抑制与恢复完整交付 |
| F03 实际 CDP 客户端框架 | 主任务已完成 | `3e697fd`；生产 protected bridge 的默认 context ID、Puppeteer Runtime 完整有限流程、Playwright 真实下载初始化拒绝、子进程/原 peer 回收；[证据](validation/cdp-client-workflow-2026-10-09.md)。符合原 F03 可复现结果/缺初始化语义时的协议依据与方案要求；F04 各域及真机验收继续 |
| A03 有限 Rust device query | 主机子项已验收 | `6f582c9`；fresh socket/DeviceID、单一总期限、wire/metadata/owned output 预算、TCP/Unix/TLS 与原 owner 退役；[证据](validation/sdk-device-query-2026-10-09.md) |
| L04 manager/native 资源基线 | 主机子项已验收 | 同 fixture 修改前后 1/4/16、30s/104轮测量和独立60s分配器/RSS归因；[证据](validation/manager-resource-baseline-2026-10-09.md)。吞吐/文件/服务流/其他平台与真机继续 |
| A03 Python/C 常用 SDK | 主任务开发交付 | 设备/应用/文件/日志句柄，Python context/async/Tunnel扩展，C checked释放、状态分类与双语言示例集中交付 |
| Q02 真机回归矩阵执行适配器 | 主任务开发交付 | 真实设备发现、前置条件与范围校验、矩阵用例命令适配与HMAC脱敏交付；[证据](validation/real-device-regression-adapters-2026-10-10.md) |

真机测试按用户要求留待其后续统一安排，相关任务不标完成。当前外部缺口：没有已指定的授权设备、iOS/USB/Wi-Fi 状态及对应 workflow fixture，
回归入口的 22 项真机行仍为 NOT_RUN；缺 macOS、Windows/UNC 和其他发布目标的
实际运行证据。恢复执行还缺明确机型、固件、模式转换和真实协议证据；不能用
现有 codec 或离线测试替代恢复循环。RSD/remoted 共存、实际 DDI 可用性、媒体/HID
和富内容剪贴板按原任务分别待条件。doctor 的 OS metadata/regular file IO 不能物理取消，runtime 退出可能等待 blocking
worker；Unix FIFO 已拒绝，此限制仍需独立处理。上述条件不阻止其他主机工作继续。

主 agent 统一集成、冻结被测源码、串行编译和批量验收。子 agent 在独立 worktree
开发或只读审查，禁止启动 Rust 编译。验收通过后本地提交；不推送、发布或部署。

当前执行：Rust query/实际 CDP 框架和 manager 路由恢复/持续 stop 均已验收并本地提交。
Python 有限 query 六文件静态包已冻结；C 句柄/linked fixtures 正在冻结前复核，
均尚未编译/运行验收。Darwin watch 在独立树承接开发。L04 同 fixture 修改前后测量与资源归因已完成：
1/4/16 规模、16 devices 的30s/104轮真实 native 启停/断连，原joins/严格EOF/
任务与FD恢复通过。独立60s诊断拆分匿名/文件页及分配器数据，末10秒RSS稳定；
验收仅限本主机子项，吞吐、文件/长时间流及平台真机继续。
Python/C尚待集中集成验收。重复制品验证的uv缓存已改为随本次验收结束清理；
旧缓存已回收，磁盘从98%降到31%，dirty worktree源码完整保留。
后续[构建缓存与worktree维护](build-cache-management.md)优先复用已有树并按交付边界验收。
