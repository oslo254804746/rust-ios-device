# L02 admitted Vec creation order · 2026-10-16

父提交 efd4dee 中，KnownVec、coordinator 和八处直接 Vec backing 路径在
try_reserve_exact 已分配之后才调用 known_metadata_charge/known_vec_backing。
这能限制保留量，但拒绝发生前已创建了未准入的临时 backing，不能证明创建前
预算。此前各子项的保留费用证据仍成立，其“创建前”表述由本次实际修复补齐。

新增共用 exact_backing：检查 Layout::array，取得原共享池 grant 后才调用 alloc；
非零分配的 Vec 使用同一 pointer/capacity/layout 释放，不假设 private allocator
rounding。零长度/ZST 不分配；布局溢出、配额拒绝或 alloc 返回 null 都不改变
旧 owner。AdmittedVec 的 values 位于 storage 前，未使用结果也先释放 backing
再退费；拆字段到局部时先绑定 storage。增长保留旧、新两份 grant，只在新分配
成功后移动元素，先释放旧 backing，再退还旧 grant。

生产接线包括：KnownVec；DeviceTable；USB unique-identity index；paired plan；
candidate index；完整 paired-observed index；USB attachment index；USB selection
index/output；native registry 固定 slots。原选择/去重/顺序/ceiling/容量规则保持。
coordinator 的 canonical key 也改为先取得 body/key 费用、复制到精确字节数组，
随后原 buffer 转 String 并合并原池 grant；原 gate、Ticket、epoch 和清理归属不变。

两处局部 lifetime 一并修复：paired-plan 结果在后续 observed-index 拒绝时，
indices 先于费用释放；scan 的 selected output 在取消或错误退出时先释放 Vec，
再释放 selection grant。IntoIter 保留原 iterator-before-fee 顺序；筛选/truncate
不提前退还仍在分配中的容量。没有改变 Source=NoDevices/生产 NativeSource。

新增八项回归在实际分配后观察 pointer/Layout/池余额：增长拒绝零分配且原指针
与元素不变；成功分配时新旧布局同时收费；溢出/空池零分配；128-byte 对齐及
元素 Drop 时费用仍在；ZST logical ceiling；registry slots 拒绝退回 body；
coordinator backing 拒绝；canonical key 第一次复制前拒绝。只在这些显式分配点
使用 cfg(test) thread-local hook，不安装全局 allocator，不用采样证明预算。

完整 tunnel518/0FAIL/2 原 ignored，strict CLI all-feature/all-target Clippy、
build/fmt/diff 全部通过。只读 manager1、Source/usbmux trap0；host5PASS/0FAIL/
22NOT_RUN，real_device_executed=false。冻结 603 个 build-source 无漂移；core/
Cargo 哈希与 efd4dee 完全相同，复用未变 core 的 XPC201/tunnel131 等既有证据，
不冒充本次重跑。受测 CLI SHA-256：
`d5a68d859ba1791ed390dcbaa1c16c2ce0ba94a44c106a154b486bdfe07611c7`。
完整日志、受测 binary、source-delta、当前及提交后 raw Git blob proof 记录在：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-owned-vector-creation`。

仍缺完整准入的真实范围：native cleanup/diagnostic selection/handle/error snapshot、
scan-history/terminal snapshot、HTTP/federation/WebSocket 输出，以及 dependency
join_all/JSON/Arc/allocator 等私有存储。这些 Vec 不能用本次已收费 backing 的
修复推导覆盖。Source 原始 USB/paired 创建端、initial 公开目录和完整 freshness
也继续；128MiB 仍非完整 heap/RSS。L02 未关闭，账本 23/6/待全审计、有效 30、
Q04 全部排除保持。只有隔离 fixture 和只读 Source/usbmux0 smoke；未运行设备/
平台/SDK/feature/MSRV 保持 NOT_RUN，仅本地提交。
