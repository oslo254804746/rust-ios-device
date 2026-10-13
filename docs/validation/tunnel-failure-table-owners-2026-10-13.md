# L02 failure 表原 owner 与字节准入 · 2026-10-13

failure 表改为真实收费 KnownVec<Box<Node>>。每个原 node 同时持 UID、row、
共享 identity route owner 和费用；查询/快照共享原 route buffer，不深复制 String。
生产候选在 Source 首次 poll 前建立 node，UID 只复制一次，使用原 Input 已预留的
一个 outcome 副本费用并保留同池 Input。writer 不再另做 intent.udid.clone。
新 Node/IdentityOwner 自有 payload 和独立 route capacity 另外准入，不能把
Source route 预留或 Ledger/receipt 各自副本的费用当作这份 route 的费用。

USB/manual node 随候选准备；paired 的 Input/node/identity/唯一 outcome key、
临时 tuple backing 及候选增长在原整批 prepare 内、任何新 automatic ticket 前
准入。拒绝按 Incomplete 处理，退款实际释放的 paired owner，保留 USB window、
原 Mobdev2 publication/backoff 和无 paired ticket 前缀的规则。

table destination 是实际分配的4096个 Box-pointer Vec，按真实 capacity 收费
（Windows x64请求32768 bytes）。USB window 在 paired 发现前准备它并跨拒绝
保留；没有 USB 候选时在 paired prepare 内准备。clear/stop 不能撤销 Source
window 已持有的独立 backing。writer 只移入原 nodes，不申请新额度；empty batch
和纯 manual success 不准备无用途 destination。

替换/成功删除/失败 pruning 全部移动原 node。late clock 在同一表和 ticket/epoch
fence 内恢复原 node、key/route pointer、version、slot 与顺序，不重建 key。
预分配 node 本身组成 RemovedBatch，无删除 key/removed Vec 的后分配。所有
writer 退出都统一处理空 backing，未消费 intents/destination 和 displaced owner
在原 guards 释放后销毁；clear 后仍由最后 receipt/Input/route snapshot owner
保持相应费用。相邻 publication HashMap 和通用 Epoch::retain 不由本项覆盖。

新增12项与原整组：462 PASS / 0 FAIL / 2 原 ignored。包括 actual retain
post-removal deadline 的零额度原对象恢复、实际 writer after-adopt late clock
空 backing 退款、真实 localhost ConnectionRefused 的原 cause 在 Ready(Err) 后
占满全部余量仍记录/保留，以及 paired 最后 Node 一字节拒绝时 USB/原 backoff
保留。原 strict EOF、native/task joins、错误 downcast/地址、停止/clear、权限、
绝对期限和及时前缀断言未删减或放宽。

首轮编译两个 cfg(test) hook 缺 Sync，41条级联错误修复后，第二轮459 PASS /
3 FAIL、第三轮460 PASS / 2 FAIL；这些结果不计交付。Root 修正新 destination
在小 paired 拒绝后挤掉 USB window 的生产顺序，并让原预算 fixture 显式计入
新增真实存储，保留一字节/零余量和原 Source 调用/发布/退款断言。新增 test helper
的 zero/overflow/stale NoChange 捷径也已删除，原 fixture 继续运行生产 fence。
全部失败日志和独立审计保留。

整组之后 Clippy 仅要求成功消费方法解包原 Box 接收值；等价消费修正后的成功
清理11项、严格 CLI Clippy/build/fmt/diff、实际只读 manager1全部通过。整组按
影响复用，不称最后源码又跑一次462。Source/usbmux trap0；host runner5 PASS /
22 NOT_RUN / 0 FAIL，real_device_executed=false。最终561个 build-source、raw Git
blob、CLI与提交关联在外部 proof；设备/其它平台/SDK/feature/MSRV 按原证据或
NOT_RUN，不据本 localhost 门禁升级。

此项只保证上述 table/node/writer 的已知存储合同。非空 manual cleanup 的
desired listing/完成列表/success payload 仍有 Source 后独立准入；无 manual 的
零额度 fixture 不能外推为整个扫描后半段无需配额。Ledger 相邻临时 identity
clone、任意 dyn Error/std IO/anyhow/Arc 私有分配、publication backing/pruning、
native scratch/report/runtime packet/socket/client 存储及完整 freshness/manual
epoch 继续审计。128MiB仍非全 manager heap/RSS 上限，L02 主项未关闭。
账本23交付/6缺开发/L02待收口、有效30和 Q04 全部排除保持。

外部证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-failure-table`。
原编译/行为失败在父目录、`round2`、`round3`；完整成功在 `final`，最终出口与
proof 在 `closeout`。只有本地提交，没有 push、部署或设备操作。
