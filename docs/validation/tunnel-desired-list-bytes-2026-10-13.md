# L02 desired listing 原 key 与 backing 所有权 · 2026-10-13

承接987c2f3的已准入shared manual route，本项把Lifecycle::desired返回的独立
String key buffer与实际Vec backing纳入同一128MiB已知元数据池。每个DesiredRow
持原key的capacity permit，KnownVec持实际capacity×inline row size；扩容期间
新旧backing同时计费。quota失败丢弃整个未发布列表，不改变原gate、desired
version、pending或watch状态，也不进入Source发现。

refresh借用列表key建立manual/suppressed索引，在所有Source/candidate/writer
等待期间保留原列表，不再复制这些map/set key。manual success批次消费列表时，
原String和同一permit移到SuccessIntent，独立于原列表iterator backing继续持有。
其它candidate/retry副本的既有owner合同保持；本项不把未准入HashMap/HashSet
bucket、Arc/watch/anyhow内部或原错误payload计成零，也不声称整个manager heap
受到128MiB限制。

新增门禁覆盖独立key/inline backing、iterator与转移owner分别最后释放、部分
列表quota拒绝且版本不变、少一byte拒绝、clear/terminal零预算空列表，以及实际
refresh在quota失败时USB/paired/connect零调用并保留Stop。旧fixture仅初始化新增
可选key permit字段；原错误、EOF、native/task joins和deadline断言保持。

新增5项门禁及受影响tunnel整组413项通过、0失败、2原ignored。首轮编译发现
borrowed suppressed key查询仍传String，改为原str借用后通过；失败日志保留。
严格CLI all-feature/all-target Clippy、CLI build、fmt/diff通过；实际只读manager
1项通过并检查Source零连接；host runner为5 PASS/0 FAIL/22 NOT_RUN，真实设备
未执行。共用冻结构建还含分别提交的SpringBoard/Doctor源码，精确来源由summary
列出，最终三项提交后再次逐文件核对HEAD，而非把混合构建冒称早期HEAD产物。
验证结果及source/binary/log hashes记录在外部summary。其它未运行门禁不计PASS；
完整L02仍需original error/history/native创建端合同、其它集合backing，以及完整
freshness/reuse/restart/manual epoch核查。账本保持23交付/6缺开发/L02待完整收口，
有效30，Q04及全部子项排除。

外部目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-desired-list`。

本轮子Agent获用户授权，仅处理独立源码/只读审查；Rust/Cargo/maturin由主线程
串行执行。没有真实设备发现或操作，没有push/发布/部署。
