# L02 scan history final owners · 2026-10-18

原 HistorySnapshot/AbandonedScan 的 body permit 位于 raw Arc payload 内，
不能证明 allocation 释放后才退款。完整引用审计未发现两类根的 Weak 或 pinning
出口；本项统一接入封闭 HistoryOwner/AbandonedOwner，覆盖 Slot、inline
HistoryRoots、Session、history/census getter 及服务器 TerminalCleanupFailure。
每个克隆通过 into_inner 消费原 Arc，最后 payload/原错误/permit 析构位于
allocation 结束之后。依据 [Arc::into_inner 合同](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.into_inner)
及[原实现次序](https://doc.rust-lang.org/src/alloc/sync.rs.html#1237-1262)，
此结论限定在本次无 Weak 的私有引用图。

原八槽上限、slot 顺序、一次冻结和旧 snapshot 不变性保持。snapshot 仍在
Source 前创建并取得实际 body grant；inline array 按新实际类型 sizeof 收费，
没有沿用旧 Option<Arc> 尺寸估计或另建 root Vec。abandoned capsule 的原 grant
仍在 Source 前整体预留；取得原错误后的 Drop 不再请求额度。没有新 sidecar、
私有 header 布局猜测、任务或时钟。Shared 的 AsRef 只借用 T，Debug/Display
转发原 T，测试地址比较不暴露 raw Arc；原终态 primary/error source 保持。

新增 2 项专项通过：实际 history/census 共享原 abandoned root 与 typed IO，
8线程最后 snapshot 释放只退 snapshot body，独立 root 保留原 IO/grant，原 IO
payload 析构时 grant 仍持有并最终单次退款；关闭银行/零配额下两个 Session
反序 Drop，旧 frozen snapshot、新 slot 顺序、共同 root 和各自 IO 指针均保留，
Outcomes/newer 结束后 older 仍有效，最后 owner 准确退款。

既有 capsule 精确准入/差一字节、八槽满表、census backing 拒绝、旧 snapshot
最后 owner 及实际 native terminal report 的同 root/原 IO/EOF/joins 断言保留。
地址检查改为同一封闭 holder 的 ptr_eq，不删除原断言。最终专项 2 PASS，
完整 tunnel 715 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读 manager
1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。

661 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`cc2a62220a46aabc749776b006e15e57c036cc6bba3b2d121126a9cd11209e62`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-history-owner\round1`。

Outcomes/其他 Freshness/Publication 等 owner、私有 Arc/header 和终态 anyhow
wrapper 精确字节、opaque error/IO、发现/SDK 原输出、公开初始 RsdHandshake
及完整 freshness/刷新/复用/重启/manual epoch 审计继续。128MiB 为已准入对象池，
不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30
和 Q04 全部排除保持。仅本地提交，无真实设备、push 或发布。
