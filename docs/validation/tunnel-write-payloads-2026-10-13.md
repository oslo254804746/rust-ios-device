# L02 failure writer自有payload创建前预算 · 2026-10-13

承接aa621e3且549构建来源/CLI hash与HEAD一致，现将WriteBatch的实际
FailureWriteOwner payload及其最多一个SourceFailuresContext payload纳入原128MiB
已知元数据bank。原三个KnownVec destination backing仍计实际capacity；在Source
连接window前先原子预留两个自有payload，再逐个取得backing，任一步失败全数
退回，不能在取得原Source错误后再因额度不足拒绝其context。

实体owner持自己的sizeof permit，共享Arc/Session/调用方最后owner释放才退款。
context未创建时只持prospective份额；terminal finish/abandon不需context时立即
退回。如果实际返回context，其实体sizeof permit与已准入receipt backing一起
移到原typed context，caller/history最后释放才退款。原error/source链、first/latest
对象、Receipt.completion、writer顺序与native joins/EOF断言不改变。

精确helper现覆盖原三个backing加两份payload预留，已有少一byte拒绝门禁仍在
对应准入点失败；没有在fixture helper中自动抬高bank。新增独立门禁检查实际
owner费用/未创建context立即退款/多个Arc最后释放，以及payload预留拒绝先于
全部destination backing。原caller错误保存与最后report owner门禁继续执行。
实际通过/失败及完整source/binary/log hashes由外部summary记录，未运行不计PASS。

相关9项门禁通过（2新增、7原保留），tunnel整组424通过/0失败/2原ignored。
严格CLI all-feature/all-target Clippy、CLI build、串行fmt/diff通过；实际readonly
manager1通过并确认Source零连接；host runner5 PASS/22 NOT_RUN，真实设备未执行。
其它未变Core/F02/SDK门禁按前项影响复用，不冒称本项全workspace/平台重跑。

Arc控制块、anyhow header/backtrace和任意原source/private/native/panic payload、
其它retry/history/native/report/map/set分配与完整freshness规则仍缺实际创建合同。
本项只交付这两种自有payload，不能据此称全部manager heap或L02完整128MiB预算。
账本保持23交付/6缺开发/L02待完整收口，有效30，Q04全部排除。

外部目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-write-payloads`。
