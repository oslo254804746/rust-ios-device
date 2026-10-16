# L02 manager record copy ownership · 2026-10-16

生产原生 record 曾用 String clone/format 创建独立输出，列表用普通 Vec 收集；
record().ok() 还会把复制准入失败误当成不存在。现在 NativeSource 的唯一输出
构造点先借用设备字段，checked 汇总所有独立文本的真实 u8 Layout，整笔同池
准入后才逐字段创建精确 backing。两个 address buffer 分别收费，interface
prefix 与 UID 一次写入，原字段、端口、userspace 可选项及序列化 schema 保持。

PaidRecord 独立保留 aggregate grant，data 在 fee 前析构。它没有 Clone、
Deserialize、可变 Deref 或移出字段的 API；生产原始 DTO 的 Clone 也移至 cfgtest，
避免通过不可变借用隐式整条复制。原始远程 DTO 的反序列化输入是独立边界，
本轮不据此声称其准入已完成。显式复制字段或创建 serde Value 图仍须各自准入。
start 回复借用 interface/address，序列化期间保留完整 record grant；输出 wire
grant 独立跟随最后 Bytes frame/slice。清空表或完成原生 join 不退还仍存活的
record 副本费用；最后副本析构才退还各自文本，列表数组另行退费。

本地 list 用 KnownVec，实际 backing 在复制首条记录前准入，权限过滤原地
retain；有效原生字段的 legacy skip 保持。get/list 的复制拒绝传递 ResourceLimit，
GET/connect 返回明确的503，避免误报404或查找另一条连接。ensure 的已有 live
记录复制失败直接结束；Source 成功后的新 record 拒绝带 CopyAdmission context，
默认路由不会接着 browse paired 或再 connect，原 ResourceLimit source 保留。
start 操作错误沿用原502映射及 effects=unknown，不宣称设备动作回滚。
自动 alive 检查只借用原设备字段，不为判活创建输出 record。原 epoch、路由、
deadline、授权复核及新连接未发表时的 native owner 收尾保持。

新增十项：one-byte-short 零文本分配、分配点全部精确布局已付、Unicode/schema/
独立 buffer、部分构造 unwind 退款、空字符串/None、借用 start 与最后 wire
slice 费用、live 拒绝不新增 Source、post-Source 拒绝不 fallback、自动 alive
检查零输出复制、实际 HTTP503，以及真实原生 workers join/strict Ok(0) EOF 后
多个独立输出仍持费。单元项中部分性质合并于同一测试，新增总数为十。

round1 编译发现一处旧 DTO/owner 直接比较未适配；round2 为546PASS/4FAIL/
2原ignored：一处新 HTTP 测试误写复数路由，另三个原生精确预算夹具未计入
新独立输出费用。修正测试地址及精确 record fee 后，原零剩余池/native/表/join
断言保留，增加取消后副本退款及最后独立副本退款断言。失败日志完整保留。
round3 为549PASS/1FAIL/2原ignored，唯一失败是既有 USB admission 边界测试的
1秒观察窗未到达 paired 等待点；同一冻结源码单独复核0.51秒通过，未调整源码、
生产期限或断言。round4 重新运行完整 tunnel550PASS/0FAIL/2原ignored；最终 all-feature/all-target
Clippy -D warnings、build/fmt/diff、实际只读 manager1 均通过，Source/usbmux trap0。
host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。609个 build-source 冻结
无漂移；core/Cargo 原始哈希未变，复用709db59上的core201/131等，不冒充重跑。
受测 CLI SHA-256：`9ac160e22fae9679d742384afbb63002c4bc2d73886aa88978970500ffde3f2c`。

完整日志、受测 binary、freeze/source-delta/current 和提交后 raw Git proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-record-owner\round4`。

federation Value graph/上游 received input、WS sink、鉴权/framework、private error/
IO/Arc/allocator/TaskCell、initial 公开目录及完整 freshness 等原条件继续。
128MiB仍为已准入自有对象池，非完整 manager heap/RSS。L02 不关闭，23交付/
6缺开发/L02待全审计、有效30、Q04全部排除不变。仅隔离 localhost/duplex 和只读
Source0 smoke；无真实设备操作、push、发布或部署。
