# L02 failure count slot body · 2026-10-18

原 failure slot 的 Arc<OwnedSemaphorePermit> payload 未从 global 字节池
单独准入，计数许可也在 allocation 结束前返回。本项将私有调用上下文、
FailedAttempt、Episode、Receipt 和写入批次中的 slot 根统一为封闭 Owner。
引用审计未发现该根的 Weak/pin/raw Arc 出口；最后 allocation 结束后才析构
原计数许可和 body grant。

Slot 包含原计数 permit 与原 global 池的实际 sizeof<Slot> body permit，
包含这两个字段的真实 payload；没有额外 guard allocation 或免费 sidecar。
新建时保持原计数拒绝优先，再准入 body，字节拒绝返还同一原计数许可。
两个 permit 均在 root 分配前取得。原计数始终为1，不把计数当字节；已有
ledger/table slot 的查找复用仍先于新建，因此闭池/零余量也能复用原根。

原4096容量、同 slot 身份、retry/退避、Source 前准入、原 IO/receipt、
Pending/Abandoned、epoch、清理和 native joins 规则保持。地址与同 root
断言统一接入原封闭 holder，没有删除旧断言；native/version/IO Arc 未改。
独立 fixture 的 slot body 从同一原字节池实际准入，只为该实际新成本增加
费用；原 receipt/selector/Removed 的零余量和差一字节边界保持，无 fresh
body pool 或任意预算 padding。

新增3项通过：body零/差一字节拒绝原计数退款、双拒绝保持原计数错误优先；
精确准入及闭池8线程最后 roots 返回原计数1与实际 body；实际 manager/
ledger/Source IO receipt 在闭池零字节下复用原 slot，clear/shutdown 后仍
保留原 IO 指针，最后 roots 准确退款。Retry存储20/global metadata17/完整
tunnel744 PASS / 0 FAIL / 2原ignored、upstream access10通过。all-features/
all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读 manager1通过；
Source/usbmux trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

失败记录保留：round1测试编译有8处旧Arc身份接口未转换，保留原比较后修复；
round2 slot3通过，retry19通过/1失败，最后receipt持有的实际slot body漏计
进断言（受测目标32字节），已按实际 body_bytes 补齐；原 IO/节点地址/零配额
回滚及最终全额退款保留。两轮后续均无生产变更，round3全部重新验证通过。

673个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`d66fb4fe3d53b0a8299e599c47abed9697493d233577964c1d4479423a53066c`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-failure-slot-body\round3`。

其他 owner、terminal/anyhow/private header/opaque error/IO、发现/SDK原输出、
公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为已准入对象池，
非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push或发布。
