# L02 native listener client-permit complete root · 2026-10-21

原 listener 在 worker 内创建连接数 Arc<Semaphore>，缺少实际控制块费用，
也晚于 listener bind 和 worker 创建。本项复用 096917a 已验证的稳定
Semaphore Root 与原 native permit 保活封装，metadata 路径在 bind/worker
前由原 bank 支付完整声明根。没有新增 unsafe、重写许可队列或新 bank。

ListenerSlots 的 metadata 分支持有原 Slots 根，client future 的
ListenerPermit 持有同一原 native permit 与根；原 MAX_LOCAL_PROXY_CLIENTS
64、try-acquire/超额拒绝并关闭 client、preamble、路由、错误、期限和 drain
规则保持。原无 bank 分支继续原 Arc/Semaphore/OwnedSemaphorePermit；
测试 helper 的原 native permit 也只封装到同一 private enum，没有替换原值。

ListenerLimits 接收已准备的许可 owner，worker 内不再创建该共享分配。
完整根只随原 listener 和最后 client permit 保活；没有新增 outer native
object 字段人为延长费用。最后原 permit 归还、Semaphore 原位析构、实际根
dealloc、原费用退款继续复用已验顺序。已完成 native shutdown 后的持久 owner
预算 helper 保持，构造前峰值 helper 仅增加实际 slot 根布局，不扩大固定额度。

新增测试观察 N-1 费用拒绝时零受控 backing、exact N 首次实际分配前整笔
扣费/原 layout 与地址、闭池 clone 的同一根、原容量0/1的 NoPermits 行为，
归还原 permit 后下一 client 可获许可，最后外部 root 结束而 native permit
仍在时费用仍扣，最后 permit 结束精确退款。原构造最后字节测试按新增实际
根公式覆盖完整前缀，保留 typed ResourceLimit、无 worker/task 基线与严格
duplex Ok(0) EOF。既有 client 限额、迟滞 preamble、真实转发、原 native
JoinError、实际 drain/join 和严格 TCP/duplex EOF 测试继续通过。

一轮 27 个命令全部通过：listener owner 3、无 mDNS
listener owner 3、channel 13、
无 mDNS channel 13、完整 tunnel 149、
stop 2、无 mDNS stop 2、
owner 32、无 mDNS owner 27、XPC
216、existing 22、prepared 13、
activation 9、proxy 4，无 mDNS current
6、initial 18、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有既有 dead-code warning，不称作严格无警告。全部 Cargo/build
由主控串行执行。

689 冻结源码无漂移，四处源码差异；Core 生产代码已改，使用本轮实际
Core/CLI 结果。实际 host 与归档受测 CLI SHA-256 一致：
14f5485874dff423ad6fe49db0dec8d548d436ef60ac16187a763ae1c9450714
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-listener-permit-root\round1
未声称 Miri、其它平台或真实设备验证。

原 stop watch、Notify/waiter/task/waker、公开 native handle Arc/Weak、
opaque report/cause/wrapper、其它 marker、SDK/发现和 metadata 副本、
HTTP/WS/配置/凭据、框架/allocator 与完整 endpoint 恢复继续原范围。
128MiB 为已准入已知对象池，非完整 heap/RSS。L02 未关闭，23交付/6缺开发/
L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
