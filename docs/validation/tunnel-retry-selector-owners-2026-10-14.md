# L02 retry selector 创建前归属 · 2026-10-14

生产 ConnectionContext.retry 携带 failure node 已准入的 shared Identity。原
route buffer 与费用跟随其 Arc owner，进入 Ledger::start 时仅借用 identity，
不再创建那个尚未归属的临时 AttemptIdentity/route String 副本。比较、admission、
deadline、retry counters 与 Source first-poll 的原顺序保持。

需要独立留存的 receipt 和 episode 仍各自拥有 key/route。每个创建端在复制
前按 key length/route length 准入真实 payload、最终 selector buffers 及 capsule；
复制后核对实际 capacity，保留精确费用，多余 permit 立即释放。episode 只有
全部费用准入后才构造 row/Box，不依赖 Source 返回后的收费。原 capsule/source
cause、history、displacement/rollback、最后 report owner 与原 fee lifetime 保持。
Arc/anyhow allocator 私有存储未由这个合同认证。

新增3项覆盖：保留4096容量的原 host 和其实际 pointer，在 exact quota 下
使用 shared identity、复制到独立 receipt/episode 后仍不变；原 IO pointer 与
最后 report 费用退款；一字节拒绝无 episode/receipt 且原 owner 费用保持；
episode 准入失败不构造 row，全部临时费用退款。既有全量生产 Source、deadline、
取消、native retirement、strict EOF/task joins 和零额度错误收尾同时通过。

冻结565个构建源文件，整组 tunnel484 PASS / 0 FAIL / 2原ignored。严格 CLI
Clippy/build/fmt/diff、readonly manager1 PASS（Source/usbmux trap0），host5 PASS /
0 FAIL / 22 NOT_RUN，real_device_executed=false。CLI SHA-256：
`6411379bab55890f831942c6638ba040423b28e7b3043f23450d743189dae083`。
raw Git blob/working source 与本地提交匹配由 postcommit-proof.json 核对。

证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-retry-selectors`。
整组测试、严格门禁、smoke、冻结/hash 与受测 binary 保留。仅本地提交，无
设备操作、push、部署或发布。其它平台/设备/SDK/feature/MSRV 按真实范围
保持原证据或 NOT_RUN。

publication 原 bucket/control、其它 route/record 副本、原 dyn Error/anyhow/Arc
private allocation、native/runtime buffer/backing 及完整 freshness/manual epoch
仍需完整合同与接线。128MiB 尚非完整 manager heap/RSS 上限，L02 未关闭。
23交付/6缺开发/L02待收口、有效30与Q04全部排除不变。
