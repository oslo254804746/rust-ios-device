# L02 scan manual/suppressed 索引归属 · 2026-10-13

扫描中的 manual HashMap 和 suppressed HashSet 虽已借用 desired keys，仍有
未收费的容器 backing，manual 还另外 clone ticket/route references。本次改成
实际 KnownVec backing 的借用索引：Manual 持 desired row references 和排序
位置，查找按 UID binary search，候选迭代保持捕获 desired 的行顺序；Suppressed
只持已准入 key 的借用引用并排序查找。原 HashMap 迭代顺序未提供稳定顺序保证。

每次增长同时保留旧费用，replacement 实际 capacity 先准入，然后移动元素。
manual rows 与位置分别准入，部分拒绝不发布 row；仍保留的真实空 backing
继续收费，索引释放时退款。索引不复制 key/route，也不创建独立 ticket clone；
其 Rust 生命周期要求原 desired listing 仍存活，原 key、route、gate/epoch owner
与费用不会因索引创建或 lookup 提前释放。Source 前完成索引，网络等待不持
全局生命周期/table 锁。原 authority、ticket fence、停止和失败 reconciliation
规则保持，未知/不完整 paired view 的 manual Wi-Fi 留存规则保持。

生产接线的原 tunnel436项通过、2原有ignored，无原夹具/断言修改。之后仅新增
cfgtest fee观察与返回类型别名，另两项新增测试通过：实际localhost生产HTTP
manual start 后检查原 key/ticket/route 引用、真实费用/释放及一字节不足部分
拒绝；生产 suppressed preflight 在一字节不足时零Source调用，并保留原stop
版本/epoch。其余436项按源码影响复用，不冒充单次438整组执行。
严格CLI Clippy、CLI build、fmt/diff通过；实际只读manager1通过且Source
连接为0；主机runner5 PASS / 22 NOT_RUN / 0 FAIL，real_device_executed=false。
其它未变Core/F04/SDK/feature/MSRV及平台门禁按影响复用或保留NOT_RUN。

这只覆盖上述真实自有索引。attached/seen/pending/planned/observed、后续 outcome
key 副本、任意error与private allocator/Arc/native/runtime，以及完整刷新规则
仍需实现，不是整个manager heap/RSS上限，不关闭L02。账本23/6/待收口、有效30
与Q04全部排除不变；未执行真实设备发现或操作。

外部源码冻结/原始日志保存于
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-scan-desired`，
最终554个build-source的冻结、gate与提交proof在`final`。
