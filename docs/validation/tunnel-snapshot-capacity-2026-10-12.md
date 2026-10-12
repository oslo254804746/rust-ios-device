# L02 Source 快照实际容量准入 · 2026-10-12

承接1e14082的原错误载荷采样，本项修复USB/paired快照仅按String长度计费的缺口。
短字符串的大reserve和空Vec的大capacity原本可在16MiB原文本预算内被保留。
现在按String.capacity计入16MiB快照文本额度，并以checked运算将Vec.capacity ×
元素size与文本容量合并检查，已知快照存储总额度32MiB。每字段4096 UTF-8 bytes、
原raw行数/唯一ID/candidate逻辑预算继续；candidate克隆的逻辑长度合同未改变。

四个生产消费点仍使用同一准入函数：扫描USB、发布USB、扫描paired和默认manual
fallback paired。检查完整输入后才匹配/克隆/接线，匹配首行不能略过后续容量。
paired批量拒绝与原错误、未知backoff、USB合法候选和stale epoch规则继续。
拒绝不新增配对、设备设置变化或强制后台操作；原native owner的收尾合同保持。

新增6项门禁：短UDID/host的capacity组合、恰好16MiB原buffer不变、空Vec空闲
存储、Vec与String合并超限、checked乘/加溢出；另复用原native fixture测试真正的
userspace owner，确认USB发布容量拒绝保留旧Arc并join被拒新owner，迟到paired
容量快照保持epoch静默和原native joins/strict peer EOF/task baseline。
原测试仅提取共同helper，原length和收尾断言均保留，没有放宽原精确边界。

受影响snapshot门禁34项全部通过（含新6项）；严格CLI Clippy通过。CLI build、
只读manager实际debug/秘密扫描与无设备发现门禁1项通过；主机runner为
5 PASS / 0 FAIL / 22 NOT_RUN、real_device_executed=false。fmt/diff通过，精确
source/binary/log/commit关联见外部summary。readonly manager跳过Source发现，
不能把此smoke当作capacity故障wire重放；容量拒绝由上述Rust/native门禁验证。
其它未变tunnel门禁复用1e14082证据，不称全量Rust/CLI/SDK重跑。

本项只限制收到后继续持有的已知snapshot存储。Source返回前分配、allocator/RSS、
HashMap backing、其它owner/error payload与统一全局字节准入仍未由本项覆盖。
私有限额是主机实现准入规则，不是Apple协议上限；此前超大reserve的输入现在拒绝。
不关闭L02，账本仍23已交付/6缺开发/L02待收口，有效30，Q04全部排除。
实际设备、其它平台、SDK/feature/MSRV仍按原运行范围保留NOT_RUN。

下一步：把已知snapshot/published metadata/error owner的并发准入与最终释放归到
统一规则，保留不透明payload的准确来源边界；再核查完整refresh/reuse/restart/manual
epoch。各可验收项验证后立即本地提交；无push/发布/部署或设备操作。

外部入口：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12eff-9622-7f01-b777-335a6d20895d\repair\l02-capacity\snapshot-capacity-summary.json`。
同目录有源码冻结、完整日志、旧CLI副本和`round-handoff.json`。
