# Tunnel 默认fixture隔离更正 · 2026-10-12

核查candidate索引门禁后发现：仅用于unit test的 `with_upstreams` 构造器仍注入
NativeSource。既有 `agent_refresh_and_epoch_endpoints_execute` 用默认local trust执行
HTTP refresh，可进入主机usbmux和paired mDNS发现。此前373/378/383及首次388项
PASS记录了实际方法结果，但没有建立全部Rust门禁的设备隔离证据。不能据这些
日志宣称没有真实发现，或将此前可能接触的具体设备/设备状态记为已观察。
这些测试启动前未核查到此默认路径，是本轮执行遗漏；原日志全部保留。

默认unit构造器现改为显式 `source::NoDevices`：USB与paired为空snapshot，connect
返回typed DeviceNotFound，不创建主机设备会话。只有unit默认构造器使用该Source；
生产CLI仍直接传入NativeSource。专门的协议/native测试继续注入原localhost Source，
原native任务/joins、strict peer EOF、deadline和原错误对象断言未删减。原HTTP
refresh/epoch JSON与状态码断言保持，替换的是原先依赖主机设备状态的fixture输入。

初次candidate索引的build/test/CLI日志以 `before-isolation-*` 保留。修正后需要串行
重新验证完整受影响tunnel、严格Clippy及CLI来源；最终结果由candidate-index summary
登记。修正后完整受影响tunnel388通过、0失败、2原ignored，原localhost native
joins/严格EOF及所有原断言保持。先前实际readonly CLI/host runner采用拒绝设备权限策略和localhost trap，
这两项独立的零发现证据仍有效，不扩展为先前全部Rust测试的隔离证明。

本轮不做任何后续真实设备探测或反向“恢复”操作。先前通用fixture是否观察了
具体物理设备及其设备端最终状态没有捕获，保持NOT_OBSERVED；不编造设备ACK、
无副作用保证或设备操作PASS。后续回合使用显式隔离 Source，L02和其它原主项
未完成状态继续。原制品、缓存、checkout和用户改动均保留。
