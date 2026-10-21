# L02 runtime stop bit in its original paid Ledger · 2026-10-21

UserspaceTunDevice 与 RuntimeStorage 原先克隆独立私有 Arc<AtomicBool>，
构造还在 listener bind 之后。其控制块与 flag 没有完整费用归属。本项删除
这一独立分配，将同一 runtime 的 stop bit 放入本来就被两者持有的原
LedgerState。完整 Ledger 根按实际类型布局派生 body_bytes，在 listener/
worker 前支付该字段；无新的根、bank、quota top-up 或晚期准入。

原 flag 初始化 false，Acquire 读取和 Release 写入保持。packet receive/
transmit、reader/writer 与 UserDevice.is_alive 都经原 Ledger 查看同一 bit，
is_alive 的原 task-finished 检查保持。request_stop 仍先设置停止、通知原
watch、再执行原 abort loop。RuntimeStorage.fail 的原顺序保持：保存原
cause -> Release stop -> 原 watch 通知。没有改变 snapshot、retry、
pending/join 计数或初始观察，也没有将一个 runtime 的停止传播到其它根。

删除原 stopped Arc 字段、构造、克隆及 fixture 字段；receive fixture 的
真实 packet 地址/额度/IO 断言不变，transmit 的停止断言改为读同一 bit。
原无 bank 分支仍使用原 unmetered Ledger，未声称因此已具备全局准入。
公开 SDK 接口和原 watch 行为保持。

新增两项测试覆盖闭池零额度的原 State 地址、多个 RuntimeStorage/root
视图、A/B 原银行隔离、无新增受控 backing 及最后根精确退款。另一项在
实际 Tokio worker 中记录原 PermissionDenied IO，等待原 watch 通知后
验证 stop 已为 true、first/latest 报告的同一原 TunnelError 与深层 IO
payload 地址/class、pending0/cleanup0，并实际 join 该 worker。原 storage
结束只归还 Ledger 根费用，两个报告保留原 identity/cause，最后报告结束
才全额退款且原 cause 只析构一次。该测试不将 Tokio task/watch 或 opaque
cause Arc 计作已支付对象。

第一轮新增 stop 完整/无 mDNS 各2项及原 owner/RSD 测试通过，严格 Core
Clippy 因测试模块位于文件中段而终止。进程终止后仅将同一测试模块移到
文件末尾，未改生产逻辑或增加 lint 豁免，第一轮日志保留。第二轮 23 个
命令全部通过：runtime stop 2、无 mDNS
stop 2、owner 32、无 mDNS owner
27、完整 tunnel 142、XPC 216、
existing 22、prepared 13、activation
9、proxy 4，无 mDNS current
6、initial 18、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有既有 dead-code warning，不称作严格无警告。全部 Cargo/build
由主控串行执行。

688 冻结源码无漂移，五处源码差异。Core 生产源码已改，使用本轮实际
Core/CLI 结果；host 受测与归档 CLI SHA-256 一致：
9dd45562f32957e8aa3a5303070752836fec2bf2d46be94699e5b925e9c0b284
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-runtime-stop-inline\round2
未声称 Miri、其它平台或真实设备验证。

watch/notify/channel 内部、公开 native handle Arc/Weak、opaque report/cause/
wrapper、其它 marker、SDK/发现和 metadata 副本、HTTP/WS/配置/凭据、框架
task/allocator 与完整 endpoint 恢复继续原范围。128MiB 为已准入已知对象
池，非完整 heap/RSS。L02 未关闭，23交付/6缺开发/L02待全审计、有效30
与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
