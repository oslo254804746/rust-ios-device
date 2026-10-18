# L02 refreshed metadata two-budget body · 2026-10-18

Freshness Metadata 的原 global/publication body permit 位于 raw Arc payload
内。实际 native 测试持有跨行创建的 Weak，不能采用 strong-only owner。
本项统一接入 paired MetadataOwner，覆盖 table、registry、RefreshProof 和
原子 swap 的全部 metadata 根。原 data 引用先于独立共享 guard 结束，最后
strong 及时释放字段，最后 Weak 结束 data allocation 后才释放 guard 及两池
body 费用。没有 raw Arc、pin 或免费 sidecar 出口。

每池 body 费用为实际 sizeof<Metadata> 加 guard 两个 OwnedSemaphorePermit
的实际 payload；两笔 grant 均在两个 Arc 创建前取得并核验原池/精确字节。
publication 的 catalogue known bytes 单独随字段持有，在最后 strong 结束。
guard 采用既有封闭 Shared，guard allocation 先结束再退两笔 permit。
Metadata 载体变更涉及的 registry/table/backing 继续按实际类型尺寸准入，
没有额度 padding 或 private Arc header 布局估计。

原 catalogue Arc/缓存指针、双池 bank 归属、原子回滚、绝对期限、原 Source
IO/receipt、native lease 和 joins 均保留。metadata 指针比较使用封闭 owner
的原 data 地址；公开 catalogue 的 Arc 比较保持。此项不改 Receipt 或外层
anyhow report、SDK catalogue、Publication Life 等独立边界。

新增2项双预算测试通过：任一池 foreign/short/long grant 六种拒绝均返还
两池原费用并仅析构原值一次；高对齐原值及真实 Vec backing、闭池/零余量、
8线程 Weak upgrade 保持原指针，字段在最后 strong 退款，两池 body/guard
持续到最后 Weak 并最终全额退款。原实际 native 两种 table/joins 顺序测试
加强为最后 Weak 前后两阶段费用断言，原 native/task baseline/EOF 保留。

双预算专项2、native freshness全组14、完整 tunnel729 PASS / 0 FAIL /
2原ignored、upstream access10通过。all-features/all-targets CLI Clippy
-D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux trap0，
host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

666个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`e71aa1aa4b45c867dfbe53bea54d123cb7895db69d1bc017a7888f966d6db650`。
日志、source delta、binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-fresh-metadata-body\round1`。

Receipt/Publication 等 owner、private header/opaque error/IO、发现/SDK原输出、
公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为已准入对象池，
非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push或发布。
