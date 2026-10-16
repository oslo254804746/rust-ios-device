# L02 native cleanup selection and census backing · 2026-10-16

原 retirement 清理在同步 stop、drain selection 和 caller-drop outcome capture
中创建未收费的 handle/key Vec；drain 的固定 key 数组还由返回的 future 持有。
本次用 registry 已准入 Entry 中的首次退休序号替代这些数组，不在清理时申请
配额，也不把 backing 改为大型 inline 数组。

issued 在每次 reserve 成功时永久递增，溢出在 Entry 插入和 Source 传输之前
拒绝；空 reservation 释放不退还序号。每个 incarnation 最多首次退休一次，
所以 retired ≤ issued，清理递增不会溢出。retire_matching、retire_key、晚挂接
的首次退休均经同一 helper，重复退休保持原 ordinal。两个计数及每个 Entry
字段的真实布局随 registry body/slots 在 worker 前准入；计数从不重置。

一个短锁内冻结 cutoff/count/capacity。后续升序逐槽只选择退休 ordinal ≤ cutoff
的原 incarnation；后来退休或槽位复用不会进入旧选择。每次等待 attachment、
记录错误和 certify join 仍持有原 Key/Arc，已消失 owner 可以跳过，selected
数量保持初始值。global/device stop 还保持原 namespace predicate，不顺带停止
其它已退休 owner；所有 native 调用仍在 registry 锁外。空集合直接跳过。

drain_retired 返回 future 前仍同步请求停止；原 serial drain mutex、notify
arming、绝对 CleanupClock、Caller Drop 和 native joins 规则保持。attempt evidence
由同一 future 的局部 mutex 拥有，两条分支借用它，移除不必要的独立 Arc 分配。
外层 future/TaskCell 私有布局仍是单独合同，不因此宣称完整 heap 已收费。

payload_outcomes 与退休选择分开，继续读取所有 phase 的原 handle，包括 active
但已产生 native 结果的 owner。两个 KnownVec backing 在复制历史/handle owner
前准入，结果 grant 跟随最后根数组；不足时 census 标记 truncated/归属不完整，
保留原 registry/Core ledger、原 typed IO、handle、slot 和清理能力。拒绝时数值
仅是已观察下界，不能证明零个未 join owner。此 readonly 归属拒绝不阻断 stop/join。

新增七项 registry 回归：固定 cutoff 排除后退休对象；已 join 后复用槽位不进入
旧选择且 selected 计数保持；重复退休/晚挂接；u64 边界仍可最终清理、下一次
Source 前拒绝；空 reservation 不退款序号；旧 namespace 不重复 stop 其它退休
owner；census 拒绝保留原历史和最后 backing grant。另有两项真实 userspace
worker/duplex 回归，验证池可用字节为 0 时原 joins/任务回收/严格 EOF，以及
active phase 的原 native IO 地址跨 census 配额拒绝、根数组释放和 join 保持。
原 manager native-census 回归同时核对拒绝的 truncated 标志和恢复后相同 census。

完整 tunnel527/0FAIL/2 原 ignored 在 round1 通过；Clippy 随后发现一处测试
err().expect 写法，最终仅改为 expect_err。round2 原生产源码全部哈希未变，
复用完整527（reuse-proof.json），变更的边界测试1项重新通过；strict CLI
all-feature/all-target Clippy、build/fmt/diff 和只读 manager1 全部通过。
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
冻结603个 build-source 无漂移；core/Cargo 未变，复用原core201/131等证据。
受测 CLI SHA-256：
`066314e8efe957473d46914ad960c874ee7c0e4f084f4cca26cf72b9eaff35a7`。
受测binary、完整日志、freeze/reuse/source-delta 和当前/提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-native-cleanup-selection`。

其它 scan-history/terminal/HTTP/federation/WebSocket 存储及 private IO/error/Arc/
allocator/TaskCell、initial 公开目录和完整 freshness 等仍继续。128MiB 非完整
manager heap/RSS；L02 不关闭，23/6/待全审计、有效30、Q04全部排除保持。
仅 localhost/duplex 与只读 Source/usbmux0 smoke，无真实设备操作或 push/发布。
