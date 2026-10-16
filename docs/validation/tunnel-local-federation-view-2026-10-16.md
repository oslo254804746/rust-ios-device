# L02 local federation borrowed output · 2026-10-16

本地root列表原来先创建PaidRecord，再to_value生成一套全新的String/Value对象图，
随后clone UID进入HashMap并创建Vec<Value>。第一份record持费不覆盖第二套图。
现在Listing直接保留已准入的KnownVec<PaidRecord>，序列化时借用字段与UID；
没有本地Value、复制key、per-device entries数组或借用引用数组的创建。

本地记录按UID用sort_unstable原地排序；root序列化按UID二分判断remote重叠，
不建合并索引。同UID只输出一个key，数组先本地再已合并的remote原顺序；
remote-only对象仍直接序列化其原Value。为保留原to_value后的对象字节，
本地root记录按既有serde_json排序map的键顺序写入，字段名、端口、Unicode与
可选字段省略不变。root key顺序本来受HashMap影响，无稳定顺序合同；本地
部分现按UID排序。权限过滤同时应用本地和remote，拒绝字段不被后续合并复活。

PaidRecord文本与KnownVec数组继续保留各自grant；Listing move/sort不重建字段，
零剩余池可将其序列化到外部已有writer。最终JSON wire仍在生产Replies中独立
准入，record与wire费用重叠，handler/Listing结束只释放record数组/文本，最后
body slice结束才释放wire。没有新增本地1MiB输出上限，原1MiB仍仅用于上游
received input。

五项新增回归覆盖原文本指针/零剩余配额、同UID local-first/remote顺序/扩展
字段及唯一key、两路scope过滤、旧排序对象完整字节/省略字段、超过1MiB的
本地root和最后wire slice独立费用。完整tunnel555PASS/0FAIL/2原ignored，
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1
通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
611个冻结build-source无漂移；core/Cargo哈希未变，复用fb89530的core诊断7/8
及严格core证据，不冒充重跑。受测CLI SHA-256：`c0d86b9c83d206850c3125c0cf3a9275ba54eb6beda755797eb14292861db033`。

完整日志、受测binary、source-delta/freeze/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-local-federation-view\round1`。

remote Value/map/key图、upstream注册URL副本/join_all以及received body/parser
scratch仍未获得完整创建前准入；这份借用视图只消除了本地第二套图，不能据此
宣称整个federation已覆盖。WS Binary/sink、鉴权/framework、private IO/error/
Arc/allocator/TaskCell、initial公开目录与原完整freshness等继续。
128MiB非完整manager heap/RSS；L02不关闭，23交付/6缺开发/L02待全审计、
有效30、Q04全部排除不变。仅本地提交和隔离localhost/duplex/只读Source0 smoke，
无真实设备、push、发布或部署。
