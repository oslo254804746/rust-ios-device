# L02 prepaid immutable scan history · 2026-10-16

Outcomes.history 原先在终止收尾时创建未收费根 Vec，并把它转给可独立存活的
TerminalCleanupFailure。此时才申请配额可能丢失已取得的原错误；serve 也可能
在已有 abandoned history 的同一 state 上重复运行，不能假设 serve 启动前没有
原错误。只把数组 inline 到未收费 terminal body 不能证明新的根 owner 已准入。

每次 Session.begin 现在为独立 HistorySnapshot body 预留并创建 Arc，发生在
Source 之前；它包含 OnceLock 的固定八根存储，费用字段在原根后释放。这个
grant 独立于 compound/AbandonedScan/cleanup 的原费用，正常完成退回未用快照。
Session 持有预付快照，AbandonedScan 不持有包含自己的快照，结构上没有环。

abandonment 在原 slots 锁内先保存原错误，按槽位顺序填入该 Session 的快照，
freeze 成功后才发布新 history_root；旧 root 在释放锁后析构。后续 history()
仅共享已创建的不可变 Arc，终止报告不创建新的根 backing；先前快照不会因后续
abandonment 改变。最后快照 owner 退回其 body fee；单独保留 AbandonedScan
不保留这份独立快照费用。空历史为 None，不分配空 owner。

payload_history 的只读 census 数组独立按 KnownVec 实际 backing 准入，再复制
原 roots；不足时保留原 slots/快照、返回原 active count，并由 census 标记
truncated/归属不完整。该拒绝不阻止零配额或已关闭池后的原 Session Drop/终止。
槽位上限8、原错误链/指针、原清理期限与 Source=NoDevices/NativeSource 保持。
变更仅涉及 private history accessor，未改变公开 SDK 或机器输出 schema。

新增三项回归覆盖：零配额重复读取共享同一根/原错误、census 拒绝和最后 owner
退费；两个 Session 反序结束、旧快照冻结、槽位顺序及 closed-bank 晚 Drop；
census backing 独立收费/释放。既有 capsule 用例核对 bare-original 与 snapshot
费用独立；真实 native repeated-terminal 用例在池为0时重复 serve，仍共享同一
快照及原 IO 地址。所有 runtime 只使用隔离 localhost/duplex。

外部无构建草案经 root 审阅后应用；root 修正测试断言写法并格式化，冻结后串行
运行门禁。完整 tunnel530/0FAIL/2 原 ignored、strict CLI all-feature/all-target
Clippy、build/fmt/diff 全部通过。只读 manager1、Source/usbmux trap0；host5PASS/
0FAIL/22NOT_RUN，real_device_executed=false。冻结603个 build-source 无漂移，
core/Cargo 哈希未变，复用原 core201/131 等证据，不冒充重跑。受测 CLI SHA-256：
`1f0f80924b6e071013a03d7a3c7742aa575e9b86268e8663a28d385b2bda58a4`。
原草案、manifest/notes、完整日志、受测binary/freeze/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-history-snapshot`，
以及 repair 根目录的 history-prepaid-draft.patch/notes/manifest。

本项精确覆盖 snapshot app body 和 census backing；Arc/anyhow 私有 header、
独立 terminal wrapper body、任意原 error heap、HTTP/federation/WebSocket、initial
公开目录与完整 freshness 等仍继续。128MiB 非完整 manager heap/RSS；L02 不
关闭，23/6/待全审计、有效30与Q04全部排除不变。仅本地提交；未运行设备/
平台/SDK/feature/MSRV 保持 NOT_RUN。
