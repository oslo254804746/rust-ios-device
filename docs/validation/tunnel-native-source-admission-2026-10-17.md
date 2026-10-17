# L02 original pre-Source admission failures · 2026-10-17

原default ensure只把部分已标记准入错误排除在paired fallback之外。实际填满
4096个native registry reservation后，A connect虽然没有poll，paired discovery
仍多执行1次；B原publication保留。红灯记录counts从(1,1,0)变为(1,1,1)。

现在为原Source future创建前的ownership reservation、publication destination、
retry version/start准备错误附加私有Admission context。default ensure直接返回
该错误，不把容量/epoch/目的地拒绝当成usbmux传输失败再发现。只在这些明确
pre-Source位置添加marker；Source真实错误不标记，原paired fallback、发现
条件和选择规则保持。原Error没有stringify/rebuild，anyhow context保留原typed
downcast、source链及allocation指针；新上下文只说明尚未获准运行Source。

新增4项覆盖：满registry红灯转绿，A Source/USB/paired均无新增调用、B同原
publication仍可查询、reservations不被误释放；global/published任一pool对
publication destination差一字节时零发现/Source、部分backing继续持费且clear
精确退款；原Error和内层IO指针不变；真实Source ConnectionReset仍按原顺序
调用paired发现并成功连接Mobdev2。既有真实native capacity用例继续验证原
cause字符串在source链中、原handle joins及回收，未删除其错误条件。

round1专项4PASS、完整tunnel634PASS/0FAIL/2原ignored、upstream10PASS。
随后Clippy指出新增fixture的std Mutex guard作用域跨清理await（显式drop未
消除此lint），改为词法块；round2仅该fixture作用域发生变化，生产源码哈希
完全相同，故复用round1全量/上游证据，不冒充重跑。round2专项4PASS、
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff及实际只读manager1
通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
635个最终冻结源码无漂移。core/Cargo未变，继续复用原core证据。CLI SHA-256：
`72cc89ef5c4d2c96f76742d1a55641cd72818f832bc32cb5179c6f1d45d6b0a7`。

最终证据（日志、source/round delta、受测binary和提交后rawGit proof）：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-source-admission\round2`。
red保留原多一次paired发现失败，round1保留完整通过与Clippy失败。

此项解决已证明的准备失败误fallback，不宣称所有Source/SDK错误都不能fallback。
其它Source/candidate实际创建、框架upgrade/private task joins、HTTP/WS/URI/
client/config/credential存储、initial公开RSD目录与完整freshness全审计继续。
128MiB非完整heap/RSS；L02不关闭，23交付/6缺开发/L02待全审计、有效30及
Q04全部排除保持。仅本地提交与隔离fixtures，无真实设备、push或发布。
