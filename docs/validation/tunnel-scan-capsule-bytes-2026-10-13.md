# L02 scan自有胶囊创建前预算 · 2026-10-13

承接cb08d10且548构建来源/CLI hash已经与HEAD核对。Outcomes::begin在选择空slot
后、标Active及Source首次poll之前，从同一128MiB已知元数据bank原子预留compound
context最大自有inline payload、独立ScanCleanupFailure及AbandonedScan payload。
未创建的份额是prospective reservation，不是测量到的heap，Session/Acquired等栈
对象不计费。拒绝不改变slots、ready或历史，USB/paired/connect均零调用。

SourceContext/FailureContext沿用原错误链结构，构建前从已保证的份额split精确
sizeof，退回未用的max差额；最后拥有该context的caller释放其permit。原anyhow/
source对象及独立secondary不格式化、不改写、不增加外层StorageContext，因此
既有primary_depth+1断言保持。Session::finish消费session；terminal take之后
生产不refill原source slots，take→原cleanup→finish→Drop路径不二次创建context。
ScanCleanupFailure有独立permit，可与原SourceContext同时保留；AbandonedScan
的实际Arc payload在取消Drop时接收其份额，history及外部根最后释放才退款。

消费式anyhow downcast将typed context移出旧heap时，permit随value保留到其Drop，
是保守的准入所有权，不冒称逐byte实时heap/RSS。plain original不新增自有context，
finish退回未用份额。先前scan context作为新原cause时，新scan独立按实际新context
收费，不依据某个foreign marker跳过；不需要穿透未知原error寻找计费标志。

新增门禁覆盖精确/少一byte预留、plain original无新增chain、两个1MiB原payload
的指针/独立secondary/取消、compound与single完成context最后释放、真正terminal
take/cleanup/finish、abandoned历史多owner、typed consuming extraction及foreign
旧context新收费，并有生产refresh零Source调用门禁。原global精确故障fixture
显式加入新增preflight费用，并检查对应最后退款；不自动放大bank或降低原EOF/
joins/error/ready/version断言。门禁实际结果和失败日志由外部summary记录。

新增9项通过；整组tunnel422通过、0失败、2原ignored。首轮编译的3处split参数
类型及旧tuple字段已修正；首轮Clippy的let-return及共用F02新fixture literal
unwrap已修正，失败日志保留。纯style修复后新增9项再次通过，422其它原行为
按影响复用，不重复改动旧native/EOF/assertions。严格Core/CLI all-target/all-feature
Clippy、CLI build与串行fmt/diff均通过；实际readonly manager1通过、Source零连接；
host runner5 PASS/22 NOT_RUN，real_device_executed=false。

共用冻结构建包含待独立提交的F02终态源码；source manifest、binary/log hashes
及最终两项提交后的HEAD复核分别记录，不把混合构建归于不含这些源码的早期HEAD。

完整L02仍缺任意original dyn Error/anyhow/private backtrace/panic payload创建端
合同、Outcomes本体/Arc控制块/anyhow header、其它history/native/runtime/collection
backing、write-owner/context inline以及完整freshness/reuse/restart/manual epoch。
本项没有对不透明payload采样收费或以有限槽位替代完整准入，主项不关闭。
账本23交付/6缺开发/L02待完整收口，有效30，Q04全部排除。

外部目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-scan-capsules`。
