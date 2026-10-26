# L02 封装解析与真实背压回归修复 · 2026-10-26

审查基线为 `1d98dec`，重点检查 `aa63b9a` 的新增 IP-in-IP 解析与
`64d23c8` 的 Unix actor 验证。此子项修复不代表整个 L02 已完成。

原解析器把合法内层 Hop-by-Hop 和普通分片当 malformed，读取 TCP 时又未限制在
内层声明的长度内；外层分片的首片只有完整头链时，也因不足以容纳整个内层包而
被提前丢弃。新增四例在原生产实现上实际得到 **16 PASS / 4 FAIL**；原十六例保持通过。

修复逐层限制真实 payload 范围，在内层 IPv6 头后的正确位置处理 Hop-by-Hop，
让外层首片的头链检查继续使用原分片关联与 paid packet owners；同时检查 IPv4
fragment flags，避免把片段字节当独立内层 IP 头。逐层封装和扩展头约束参考
[RFC 8200](https://www.rfc-editor.org/rfc/rfc8200.html#section-4.1) 与
[RFC 2473](https://www.rfc-editor.org/rfc/rfc2473.html#section-3.3)。

普通内层分片仍需另一层实际重组关联，现明确为 `Opaque(44)`，保留原包。
测试同时展示普通流和**含精确私有 tuple 的内层首片**仍被转发；这是一项明确未关闭
的隔离缺口，不能把这例通过计为完整隔离通过。ESP、未知 native transformation
同样没有得到完整隔离证明。未通过广泛丢弃合法流量或更换 owner 来掩盖缺口。

原 Unix “满队列 EOF”用例只关闭空闲 peer。现用真实 datagram socket 的不可写状态、
实际 writer 与桥接管道的背压作为前提；关闭原 stream 后，先观察运行时自己停止，
再调用会主动请求停止的 collector，避免 collector 替测试制造停止证据。
逆序 join 用例也先真正耗尽并关闭 byte bank，再执行 stop/join/undo 和最终退款。
新增的 `test_fd` 仅在 cfg(test) 下借用原 descriptor，用 poll 观察 readiness，
不配置真实接口、地址或路由。

本轮实际证据在仓库外：

`C:\Users\wangbaofeng\.codex\visualizations\2026\10\25\01a1736a-ccc6-78d0-9bae-82b6c174a8f8\l02-review-2026-10-26\round4`

- Windows filter **20/0**、Core tunnel **245/0**、activation **16/0**。
- WSL Linux 同一生产 actor **8/0**、filter **20/0**。
- Windows Core/CLI、Linux Core 的 all-feature/all-target strict Clippy `-D warnings` 通过。
- Darwin x86_64/aarch64 的 **native facade + platform wrapper** 目标检查通过，
  各有 49 个未使用告警；该外部 harness 没有包含整个 kernel_unix actor、manager 和
  prepared collector，不能写成整个 Darwin actor 已类型检查或实际运行通过。
- 同轮 CLI **1925/0/13 原 ignored**、实际 build、fmt/diff、只读 manager **1/0**
  （Source/usbmux trap=0）和 host **5/0/22 NOT_RUN** 通过。
- 780 个 Cargo/crates 输入在该轮冻结；命令、原日志与结果 JSON 逐门禁保存。

以上 Unix 用例不补足 pending ACK/响应增长、cleanup body panic 恢复、最终 owner
Drop 及全部迟到 native 边界；全 owner/freshness 审计仍需继续。
128MiB 仅指已准入已知对象池，不是整个 heap/RSS 上限。
