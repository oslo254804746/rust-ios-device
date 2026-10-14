# L02 manual success Source 前准入 · 2026-10-14

扫描在首次 USB Source poll 前准备完整 4096 项 success-only intents、Completed
真实 Vec capacity，以及 Box<Preflight>/writer/context payload。即使起始没有
manual gate，也准备这组存储，以覆盖 Source 期间新增的 UID 或 desired version。
收尾不再重新分配 desired listing、UID、receipt backing 或 failure destination。
success-only receipt capacity 为零，所需费用由实际布局与 capacity 计算。

原 canonical String 移入已收费的 DeviceGate，table 只保存 Arc；原 key buffer
和费用跟随最后 ticket/gate。收尾从当前 coordinator 获取同一锁序下的 exact
epoch/Manual desired/pending tuple，再借用 canonical key。不会预先持有全部
gate，idle Automatic namespace 的 pruning 保持原规则；初始 desired listing
仍服务原 discovery/选择语义，没有用旧列表替代晚期收尾视图。

writer 先取得 authority revision，再进入 ticket fence；原 failure version
匹配后才移除原 node。变更前后检查原绝对期限和 authority clock，晚期限恢复
原 node、failure stamp 与费用。pending clear 同样使用 exact desired/failure
Arc、authority/ticket fences 和前后 clock 检查，过期、撤权、stop、stale version
不会清除较新或尚未处理的 pending tuple。原 primary/cleanup error、backoff、
NativeLease/runtime、严格 EOF 和 task/native join 合同保留。

新增12项覆盖真实最大 backing exact/一字节拒绝、canonical key 最后 ticket、
当前新 gate/最新 version 零余量收尾、idle pruning、stale replacement、授权撤销/
过期、stop、真实 pending clear/late writer 回滚、真实 Source 回调新 gate 与
最新 pending、Source 前一字节拒绝。真实 Source 用例保留原 IO cause/receipt、
连接次数与最后 owner 退款断言，不只检查计数或旧对象未改变。

首次全量478 PASS / 3 FAIL / 2原ignored：旧额度 fixtures 未计入新预留费用，
拒绝提前发生或退款断言不符。按实际 Preflight::bytes 修正阶段后，预算17项
全部通过；原 Source 次数、原 publication/intent/backoff 和退款断言保持。
round2 strict Clippy 的仅持费字段改名为 _key_storage，最后 owner fixture 保留。
final 目录续轮中断，没有测试退出结果，按不完整运行保存，不计 PASS。

最终 closeout 冻结565个构建源文件：整组 tunnel481 PASS / 0 FAIL / 2原ignored；
strict CLI Clippy/build/fmt/diff、实际 readonly manager1 PASS，Source/usbmux trap0；
host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。CLI SHA-256 为
`ca412b5742afb8ac0f16a9878039d0fabb587cb606cec94d12b1d465d781d8d8`。
源码/raw Git blob 与本地提交关联由外部 postcommit proof 核对。未运行设备、
其它平台、SDK/feature/MSRV 保持原证据或 NOT_RUN。

证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-manual-preflight`。
首次与 round2 失败日志、final 不完整日志保留，完整成功日志/冻结/binary/proof
位于 closeout。仅本地提交，无设备发现/恢复/配对/安装、push、发布或部署。

publication 原 HashMap bucket/control、相邻 retry identity 临时副本、原 dyn
Error/anyhow/Arc/private allocation、native/runtime 实际存储与完整 freshness
仍缺创建端合同或完整接线。128MiB 是已收费自有对象池，不能称完整 manager
heap/RSS 上限。本子项不关闭 L02；23交付/6缺开发/L02待收口、分母30、Q04
全部排除保持。
