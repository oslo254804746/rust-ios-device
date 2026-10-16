# L02 remote owned JSON destinations · 2026-10-16

上游原路径用普通Vec收集HTTP body，再解析serde_json Value，随后转成HashMap/
Vec并为每条记录复制UID/upstream字段。最终wire费用并不覆盖这些输入图。
本轮保持同一HTTP/TLS/credentials/status/hop/1MiB received limit路径，通过
generic listing_body把每个chunk写入实际backing先准入的KnownVec<u8>。old/new
growth峰值同池保留；reqwest已创建的私有chunk不据此算入覆盖。

DeserializeSeed直接建立私有paid_json树。Text按真实u8 Layout先收费再复制
decoder给出的str；数组和对象entry的真实KnownVec backing在child创建前准入，
对象key文本自身独立准入。原JSON number/null/bool保持inline，无额外Box/node。
树无Clone/普通Deserialize/into_inner，data先于fee析构；输入body在parse期间仍
持费，独立decoded文本和树backing直到最后树owner才退费。decoder的escaped
string scratch在visitor前就已存在，仍是明确独立缺口，不称完整parser零分配。

对象按到达序号顺序收集，再用无heap scratch的sort_unstable按key/ordinal原地
排序，dedup保留第一份key和最后完整value，与原Value相同。避免逐key有序
insert在倒序输入下的平方级移动。partial语法/准入失败释放已建树，ResourceLimit
原anyhow cause通过slot保留，不被serde custom文字替换；syntax保留原serde_json
Error，shape拒绝仍为serde_json Error类别。原递归深度、Unicode/surrogate、
数字和trailing规则沿用锁定decoder，没有新增JSON wire限制。

注册URL snapshot的真实数组及每份URL文本先收费再复制，remote source数组
也在网络工作前预留。合并保留每个paid root与原URL，不再clone UID/生成新的
entries Value图或为upstream字段创建String。权限过滤先释放被隐藏的本地/remote
节点；随后独立预付实际Key descriptor数组，顺序收集、原地sort/dedup索引，
序列化借用原节点并覆盖upstream字段。未知扩展/嵌套值保持，local-first与注册
upstream顺序/原数组顺序保持；旧merge忽略非object条目与空组的规则保持。
原target lookup仍可在无interface时选中首个scalar，显式interface选object并读到
尚未overlay的原nested upstream，不能把listing渲染规则套到target选择。

十项新增覆盖duplicate escaped key/Unicode/NUL/数值/扩展、one-byte-short零目标
分配/原ResourceLimit、实际数组布局预付/partial退款、原深度/surrogate/trailing
及语法Error、输入销毁后独立文本、shape typed Error/空值零backing、8192倒序
key及迟到重复的最后值、merge index一字节拒绝/完整退款、实际loopback root
合并与原target语义、URL snapshot拒绝在HTTP前及body拒绝在目标分配前。原
local borrowed五项按新paid remote owner适配，保留全部语义/省略字段/大输出/
最终wire独立费用检查。

round1编译发现新测试误用未启用的Response::json；改用bytes+serde_json。
审查同时修正schema拒绝保持JSON错误类型。round2完整tunnel564/upstream6
通过，但Clippy发现旧HashMap导入已仅供测试；移至cfgtest，未新增lint例外。
审查发现的对象/index逐条排序插入一并改为O(nlogn)原地排序，新增倒序回归。
最终round3完整tunnel565PASS/0FAIL/2原ignored、上游访问6PASS、all-feature/
all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；Source/
usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
614个冻结build-source无漂移；core/Cargo哈希未变，复用fb89530的core诊断7/8
及严格core证据，不冒充重跑。受测CLI SHA-256：`41f193642d09b23d3f9125a73a98a8fae199026bb77c11222b95098d2e422aa4`。

完整失败/成功日志、受测binary、source-delta/freeze/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-remote-json-owners\round3`。

仍缺：decoder scratch、HTTP私有chunks/框架布局、join_all future/result/waker
存储、RemoteTarget三份独立文本、其它WS Binary/sink、鉴权/framework、private
IO/error/Arc/allocator/TaskCell、initial公开目录及原完整freshness等。仅已列出的
自有存储完成接线，128MiB仍非完整manager heap/RSS。L02不关闭，23交付/
6缺开发/L02待全审计、有效30、Q04全部排除不变。仅本地提交及隔离localhost/
duplex/只读Source0 smoke，无真实设备、push、发布或部署。
