# L02 retry Ledger final owners · 2026-10-18

Ledger 原 body grant 已在创建前取得，但位于 raw Arc payload 内，不能证明
allocation 结束后才退款。完整根审计仅发现 manager.retry 和 Attempt.ledger，
未发现 Weak、raw Arc 或 pinning 出口；本项统一采用封闭 LedgerOwner。
所有克隆通过 into_inner 消费原 Arc，最后 payload/State/body permit 的
析构在 allocation 结束后执行。此合同仅覆盖本次无 Weak 的私有引用图，
不包含 private Arc header 的精确字节估计。

工厂仍从原池预付实际 sizeof<Ledger>，原 grant、表 backing/节点/selector
准入保持，不新增 guard allocation 或预算额度。start 改为稳定的 owner
专用实现，继续把原根交给 Attempt；原准入、版本、绝对期限、失败退避、
Source typed IO、completion、取消及 native joins 规则不变。

新增2项通过：零/body差一字节拒绝及精确 body 准入，8线程原根并发释放
单次退款；实际 ticket/Attempt 在外部 Ledger 结束后独立持有原根，关闭
银行/零余量下 close/finish 不再请求额度，最后 Attempt 仅退 Ledger body，
独立快照继续保留原 typed IO 指针、Pending completion 和自身费用，最后
snapshot 结束后全额退款。未用 writer 的 Source receipt 保持原 Pending，
没有为了测试把 close 改成 Recorded/Abandoned。

专项2、完整 tunnel725 PASS / 0 FAIL / 2原ignored、upstream access10通过。
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读
manager1通过；Source/usbmux trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。前项权限热更新用例本次完整回归也通过，首次
未确认原因的502记录仍保留在 manager 初始化证据中。

664个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`c1fdee4e998a0f10143b7dbde191f390ab364e824ffd541fd1b7b8d58a04a3f6`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-ledger-owner\round1`。

Registry/其他共享 owner、private header/opaque error/IO、发现/SDK原输出、
公开初始RSD、HTTP/框架任务和完整freshness等继续审计。128MiB为已准入对象池，
非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push或发布。
