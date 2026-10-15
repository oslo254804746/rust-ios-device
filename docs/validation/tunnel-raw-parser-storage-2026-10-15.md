# L02 raw H2/XPC buffers and borrowed arena · 2026-10-15

当前prepared RSD生产路径把原manager bank原样传入checked bootstrap。收到H2
header后、读取payload之前按真实Layout准入；demux增长以真实所选capacity
预付replacement，原buffer/slice/clone在最后owner消失前继续持费。RawFrame
转入pending DATA/Bytes时转移原费用；返回slice可在framer销毁后继续持有。
chunked XPC body先准入真实body allocation再开始复制，不用未计费BytesMut
拼接；FILE_TX的大body仍按原H2 chunk limit读取。Nonebank保留原BytesMut/
Vec/decoded parser路径和原API。

prepared decoder先无分配验证/count，再准入实际Node/Edge两数组；字符串和
key借用原收费body。原bytes通过两遍解析后直接创建收费PreparedRsdCatalogue，
不创建IndexMap<String,XpcValue>树或full header+body副本。字典重复key保持
首次位置/末次value；原深度/集合/flags/bodylimit/整数signedness/UTF8/NUL/
padding/object版本、忽略container长度和trailingbytes等行为保持。准入失败
为原typed ResourceLimit终态，不重复拨号覆盖为EOF/timeout。

复用原Direct/Queued/Legacy/Passive顺序、fresh connector和caller absolute
Clock/stage budgets；没有新设相对总期限或替换native owner。当前目录的SDK
resolver/原lease缓存交换/原initial_rsd_probe/历史rsd()语义保持。

根线程审查修复草稿中的Standalone Vec/fee析构顺序：raw frame读取取消/IO失败、
parser第二数组准入失败、catalogue特征/行复制失败、selection重试等均以费用
先绑定、真实数据后绑定，保证逆序局部析构先销毁数据再退费；最终struct仍是
数据field在permit之前。原零剩余额度和fullrefund只能证明最终状态，本项另以
实际元素Drop在早退与panic时断言fee仍持有，避免只验证最终退款的盲点。

新增17项：4个actual raw payload/growth/最后clone-slice；4个borrowed arena/
duplicate/nativeUTF8/第二数组rollback；7个真实H2四mode/invalidPort/绝对过期/
生产IPv6localhostconnector/离开framer的bytes/partialframecancel/finalrawquota；
根线程另加earlyfailure+unwind原element析构持费，以及嵌套全部类型、逐截断/
每字节变异和trailingbytes与原decoder对照。最后两项是新增于原15项之上。

初轮XPC138PASS/1FAIL：新四mode fixture在发送目录后过早drop peer，客户端
后续bootstrap写入得到BrokenPipe。修正为同一peer保持到原client严格EOF，
未改变生产协议的写入/错误处理。第二轮根新增differentialtest括号错误保留
日志；修正后XPC141、prepared4、existing18通过。第三轮strictcore仅3项
新test err().expect lint失败；等价改为expect_err后最终冻结复验。

最终冻结589个build-source，XPC141/prepared4/existing18/core131/CLI504
均通过，0失败/2原ignored；strict core/CLI all-feature/all-target Clippy、
CLI build/fmt/diff通过。无默认feature和无tunnel的dproxy+fetchsymbols+restore
all-target编译通过（保留实际cfg deadcode warnings，不称这两组strict通过）。
readonly manager1 Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。CLI SHA-256：
`73d41030bde42818fc7ddac142e9c14c81f61b781e97c33e94d81a1b0df67ad4`。最终门禁/受测binary与source/hash在closeout。原失败日志及精确sourcefreeze/受测binary/hash/postcommit证据
位于：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-raw-parser-storage`。
独立草稿/原bytes/hash/Bytes1.11.1依赖源码仍存于相邻draft目录，根线程按目标
raw hash先检查再接入并自行修复/验证；draft NOT_RUN不代替实际门禁。

未完成合同：Bytes::from_owner私有Box header的析构在public owner之后，不能
在owner内部猜测header费用或过早退款；H2 HashMap/HashSet/VecDeque backing/
scratch、outbound XPC/H2编码对象、raw operation futures、Tokio/task/JoinSet/
waiter/watch/oneshot/Arc/privateerror/report/allocator与其它路由副本仍继续。
初始可选RSD观察仍使用历史目录API，不能因当前prepared路径计费就称全部
initial/SDK/raw parser路径已收费。128MiB不是完整manager heap/RSS上限。

只本地提交，无设备发现/恢复/配对/安装、push/发布/部署。设备/其它平台/SDK/
MSRV/未测feature保持真实NOT_RUN。账本23交付/6缺开发/L02待完整审计、
有效30与Q04全部排除不变；本项不关闭L02。
