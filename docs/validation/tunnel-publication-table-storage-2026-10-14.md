# L02 publication 真实存储与原 node · 2026-10-14

唯一生产 publication 表改为 Vec<Option<Box<Node<ManagedTunnel>>>>，取代无法
观察真实 bucket/control 布局的 std HashMap。slots 为既有4096 gate ceiling的
真实完整 backing，row/key 为真实 Box<Node> 与 String buffer；它们在 Source
连接前取得同一 global128MiB与publication64MiB两组费用，再按确定布局分配。
这份 Vec 就是生产表，并非另存未使用的估算 destination。ManagedTunnel/key
从原 shared metadata fee 拆出，保留 device/route 动态对象费用，不重复收费。

prepare 持 prospective node，publication 不再分配 key/backing。并发出现原
row 时复用其原 node/key，保留旧 value/incarnation；晚 clock 回滚原 value、
node、key buffer、slot 与费用。拒绝对象和 displacement owner在table/ticket/
authority guards外销毁，不创建或更换 NativeLease。prepare先收费再分配，
并保持原 Retry/Source first-poll、绝对期限、selection/manual epoch规则。

paid node 已包含 intrusive removal link/original slot。Epoch prune不再创建
临时 Vec或key副本，零剩余额度仍能移动原nodes；late clock、stale epoch和
predicate unwind恢复各原slot。list_records/get_record/clear/server Owner也
返回原 removal/whole owner，在table guard外析构。删除最后row后，仍实际
allocated的slots保持收费；clear移动整个backing，最后实际owner释放后退款。
原native/device共享费用与fresh catalogue仍按原native joins/cache last owner
持有，不将已释放table存储误算为尚存native对象。

7项table门禁覆盖真实dual-bank布局/最后owner、backing/node一字节拒绝、
原pointer/incarnation、zero-quota replacement rollback、clear后generation无ABA、
guard外析构与predicate unwind。3项Epoch门禁取代原5项临时Vec门禁，另有
真实userspace原runtime/zero-quota removal以及shared/sub last-byte Source0测试。
原HTTP expiry fixture携带整原Removed node并复用生产restore，而非新key/lease。
strict peer Ok(0) EOF、typed original error、task baseline与native joins保留。

外部草稿最后三处Fake payload修改在manifest冻结后发生，首次guard拒绝；
反向复原三处逐一匹配原proposal SHA后才接收。重基fac4981无冲突；root另将
先allocate后收费改为prepaid exact布局，并增加node末字节门禁。10文件guard
全部通过才一次应用。首轮2个旧fixture API编译错误保留；round2运行500 PASS /
4 FAIL / 2原ignored，均为真实backing退款、paired目标额度阶段与caller-drop
锁阶段不符。按实际owner/stage迁移后，round3完整tunnel504 PASS /0 FAIL /
2原ignored；最后仅测试方法加cfg标记后publication46项复验通过。

最终冻结574个build-source，strict CLI all-target/all-feature Clippy/build/fmt/
diff通过，readonly manager1 Source/usbmux trap0通过；host5 PASS /0 FAIL /
22 NOT_RUN，real_device_executed=false。原Core/SDK/其它feature/平台/MSRV
未改或未重跑部分按原证据/NOT_RUN保留。CLI SHA-256：
`cea657f7cebcf509847daf02ac57114167b619a46e982237688dc6f2bdac60d0`。
源码/raw Git blob与提交关联由最终postcommit-proof核对。

证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-publication-table`。
原失败/各round完整日志保留，完整504在round3，最终46/strict/build/smoke/
冻结/受测binary/proof在final。外部原proposal/drift/rebase/root guard在相邻
l02-publication-table-draft与l02-publication-table-rebase-fac4981-reviewed。
只本地提交，无设备发现/恢复/配对/安装、push、发布或部署。

本项关闭publication原bucket/control未知布局的实现缺口。其余 route/record/
SDK返回副本、manager/Arc/anyhow/std IO/future/private创建端、raw H2/XPC和
native Tx/socket/chunk/queue/runtime存储与完整物理刷新规则继续。128MiB仍
不是完整manager heap/RSS上限；本子项不关闭L02，不升级23交付/6缺开发/
L02待收口、有效30的账本，Q04全部排除保持。
