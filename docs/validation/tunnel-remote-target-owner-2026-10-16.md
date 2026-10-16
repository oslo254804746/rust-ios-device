# L02 remote target text owner · 2026-10-16

remote_target原返回三份普通String：已注册URL再次复制，以及选中记录的interface
和下一跳upstream。解析图持费不覆盖这些返回后继续存活的独立文本。
现在RemoteTarget没有Clone，三个字段使用不可变paid_json::Text。已准入URL
snapshot直接move到target并保留原指针；可选文本按各自精确u8 Layout先准入再
复制。None与Some(empty)保持。部分复制拒绝释放已建字段和原URL，原
ResourceLimit cause继续返回，不改成None、缺失或另一次target尝试。

每个Text的data先于grant析构。返回值不依赖输入body/解析图/collector/state
生命周期；目标URL与选中可选文本各自持费至字段销毁。原WebSocket URL的
ws/wss、path、UDID/port/hop/interface/nested upstream参数规则保持，consumer
仍借用原target跨await拨号。独立Url/request/header/握手/WS私有复制不由原
target grant覆盖，继续列为缺口。

新增三项覆盖原URL指针move和完整参数/可选项、最后一字节拒绝时部分字段
精确退款/原ResourceLimit、实际loopback选择后body/tree/source数组已退款而
target在manager clear/drop后继续持费并最终精确退款。既有root合并测试明确
drop两个已核对的paid targets后再检查请求退款，未弱化金额断言。
完整tunnel568PASS/0FAIL/2原ignored、上游访问6PASS、all-feature/all-target CLI
Clippy -D warnings、build/fmt/diff及实际只读manager1通过；Source/usbmux trap0，
host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。615个冻结build-source
无漂移；core/Cargo未变，复用fb89530 core诊断7/8与strict证据，不冒充重跑。
受测CLI SHA-256：`7dd3fd3ad1eccd461cf8ccea5866e6be76277299cfd05850c8b8ae00d79ca682`。

完整日志、受测binary、source-delta/freeze/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-remote-target-owner\round1`。

join_all future/result/waker、decoder scratch、URI/header/client/registry/policy/
auth/framework/WS/private IO/error/Arc/allocator/TaskCell、initial公开目录及原
完整freshness等继续。128MiB非完整manager heap/RSS，L02不关闭，23交付/
6缺开发/L02待全审计、有效30、Q04全部排除不变。仅本地提交及隔离localhost/
duplex/只读Source0 smoke，无真实设备、push、发布或部署。
