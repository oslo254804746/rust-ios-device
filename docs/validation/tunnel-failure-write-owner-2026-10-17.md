# L02 FailureWriteOwner final allocation · 2026-10-17

原 FailureWriteOwner 将 body permit 放在 raw Arc payload 内，最后 payload
退款先于 allocation 释放。完整引用审计未发现该类型的 Weak、pinning 或
原 Arc 出口；本项将 WriteBatch、owner() 及 Session.writes/retain_writes
统一接入封闭 WriteOwner=Shared<FailureWriteOwner>。每个克隆都通过 into_inner
消费原 Arc，原 allocation 结束后才执行最后 value/custom Drop/permit 析构。
该无 Weak 路径依据 [Arc::into_inner 合同](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.into_inner)
和[原实现次序](https://doc.rust-lang.org/src/alloc/sync.rs.html#1237-1262)。

没有新 sidecar、unsafe、深复制、任务或准入请求。WriteOwner 指针宽度、
FailureWriteOwner/WriteState 的原 payload 和费用不变；不把 Session 的栈
布局当作 heap/RSS 证明。所有 owner/context/backing 仍在 Source window 前
预付。Session 后续 discovery/ready 失败或 caller Drop 仍持有同一 owner 和
原 receipt backing；最后 custom Drop 只按原规则完成 unfinished receipts，
已 Recorded/Failed 的状态不被改写，in_scan/finished 规则保持。

新增 3 项专项通过：实际 Ledger 原 IO 经 FailureIntent/WriteBatch 进入原
Session，batch 结束后 Session 继续持根，在零余量下完成原 completion error，
返回相同 receipt/source IO/completion IO 指针及 Failed(Timeout)；8线程最后
WriteOwner 克隆在零配额下仅将 Pending 标为 Abandoned，保留原 Recorded/Failed，
精确释放 body/context/实际 backing，原 IO 最后报告保持；Session caller Drop
不丢原 receipt，不虚构 discovery history，最后 root 结束后全额恢复。

首轮新 fixture 漏导入 Node 导致编译拒绝，进程结束后补齐 import，原测试
断言和生产行为未变，日志保留。最终专项 3 PASS，完整 tunnel 713 PASS /
0 FAIL / 2 原 ignored，upstream access 10 PASS；all-features/all-targets CLI
Clippy -D warnings、build/fmt/diff、实际只读 manager 1 项通过。旧精确预算、
same-poll 原错误、writer/whole-scan 失败、native handle/实际 joins/严格 EOF
断言未放宽。

Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
660 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`4047d6d1ed8939cb910d4dfc92544309864b4ab06c56488077b59341cbf8c652`。
最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-write-owner\round2`。

scan/history、Freshness/Publication 等共享 owner、私有 Arc header 字节、opaque
error/IO、发现/SDK 原输出、公开初始 RsdHandshake 及完整 freshness/刷新/复用/
重启/manual epoch 审计继续。128MiB 为已准入对象池，不代表完整 heap/RSS；
L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30 和 Q04 全部排除保持。
仅本地提交，无真实设备、push 或发布。
