# L02 pending DATA actual ring backing · 2026-10-15

同一manager bank沿原checked H2 constructor进入pending DATA owner；Nonebank
仍使用原VecDeque。metered路径使用显式Vec<Option<DataFrame>>环形slot，按
真实Layout在创建前准入。首次4slot、后续双倍动态增长，增长期间旧/new backing
同时持费，不猜std私有deque布局，不另加截断或计数限制；原1024帧协议限制
和原stream/byte预算保持。

原DataFrame/Bytes/flags在slots间直接移动，不复制raw payload或替换其owner。
退队和按stream清理保持FIFO，clear继续保留实际backing及其费用直到队列
销毁；已返回frame/slice/clone可在framer销毁后继续持原payload fee。旧backing
销毁后才归还旧fee，新backing失败不触碰原slots/原帧/原head/原fee。
原pending→demux失败把同一frame退回front时必有原空位，不增长也不重新收费。

原协议验证/计数limit和逻辑reserve先于新backing准入；准入拒绝先回滚本次
payload的逻辑字节，原typed IO ResourceLimit回到原H2Operation.finish，已
读header/body则沿前项guard失效。未把后续invalidating清理当作本次rollback。
原replenish/window/END_STREAM/reset/GOAWAY、同一reader、绝对deadline和
native错误/严格EOF规则保持，无独立task或新timer。

新增9项：3000步与原deque对照的FIFO/wrap/pop/frontrollback/streamclear；
真实old4+new8峰值末字节拒绝保持原slot指针/帧与随后exact增长；原payload
slice离开ring且clear仍持backing；零配额frontrollback/vacant slot复用；
实际阻塞writer读入三帧业务DATA/flags并在flowcredit后原样返回、payload最后
owner；实际第5帧growth拒绝命中真实8slot分配点、原typed fatal/invalidation/
旧backing持费与strictEOF；pending→demux失败零配额退回同一buffer/pointer/
逻辑账本后重试；原guard执行前验证新frame quota失败逻辑字节回滚且原stream
buffer保持；零配额时原stream0协议错误先于新queue准入。

最终新增9项、XPC158/prepared4/existing18/core tunnel131/CLI tunnel504
均通过，0失败/2原ignored；strict core/CLI all-feature/all-target Clippy、
CLI build/fmt/diff通过。无默认feature和无tunnel raw组合(dproxy+fetchsymbols+
restore) all-target编译通过，保留实际cfg deadcode warnings，不称这两组strict。
readonly manager1，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。冻结592个build-source，当前源码无漂移；CLI SHA-256：
`bc15dfe2f8ebd9f5ecac24af058f2b15371011f0c8217b956ff1db792245e774`。受测binary/完整门禁/sourcefreeze和提交raw
Git blob/工作区proof均在证据目录。证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-pending-data-storage`。

H2其它maps/sets/tombstonequeue/outbound编码/initialRSD、Bytes私有ownerheader、
Tokio/JoinSet/waiter/Arc/privateerror/report/allocator及原full freshness继续。
128MiB仍非完整manager heap/RSS上限。本项不关闭L02；账本23交付/6缺开发/
L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备操作/push/
发布/部署；未运行平台/SDK/feature/MSRV/设备保持真实NOT_RUN。
