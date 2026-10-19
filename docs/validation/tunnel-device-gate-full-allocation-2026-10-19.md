# L02 DeviceGate complete shared allocation · 2026-10-19

原 DeviceGate 与 UID 缓冲区共用原 metadata grant，但 std Arc 控制块未计量。
本项改为应用声明的精确共享分配；完整 counter/value/原 grant/padding layout
加原 exact-capacity UID 字节在复制/创建前同池准入。原 key allocator 与表增长
顺序保持，原组合 grant 在 root 内随原数据保留；不创建新池、sidecar 或 key副本，
不估算 std Arc 私有布局。

DeviceGate 仅移除原内嵌 grant，DeviceRow 使用现有 exact shared with_fields。
原 serial Mutex、watch epoch、intent Cell、同设备查询/停止/clear/重用规则不变。
原 pruning 判定仍为 strong_count>1 || persistent，4096 上限和 live/stopped/idle
保留规则保持。new helper 的同一实际强引用计数用于生产判定，旧未使用计数 helper
删除。此处计量具体 gate/UID；watch channel、epoch Arc<()> 与其它 opaque 框架
分配仍是独立待审计边界，没有伪称它们包含在固定 layout 中。

增强原 gate-owner 测试，保留全部原生命周期断言：真实 allocator 观察到恰好
三处 backing（原 UID、两行表、完整 root），原 key/table 地址一致，原值位于
正确实际 root 范围内，不猜 offset。UID 创建前 gate+UID 额度已取得，剩余仅
原表 backing；后续表/root backing 前其整份费用已取得。同一原 bank/grant 的
完整数额正确。旧票据8线程保留同一 gate/key，关闭池后 clear 使旧票据失效，
同一 gate 重用的新票据有效；strong count3/2/1、4096剪枝、持久停止状态保留、
最后 owner 退款、短额度零复制、容量增长回滚与等待取消断言均保留。

生命周期21、严格CLI all-features/all-targets Clippy -D warnings、error payload9、
完整CLI1742PASS/0FAIL/13原ignored、build/fmt/diff及实际只读manager1通过。
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
Core/Cargo未改，既有Core证据复用，不声称重跑。未新增仅镜像实现的测试。

首轮生命周期21项通过后，Clippy对观察器显式drop后的同步锁作用域提出告警；
将原观察断言置于独立词法作用域，确保 await 前锁已离开作用域，没有改变生产
代码、quota 或删除断言。第二轮全部9个串行门禁通过，失败日志保留。681个冻结
源码无漂移，4处源码差异；受测 binary 已归档，SHA与实际host报告一致：
`15948ecb7b011e925b1cc01bcfa7032cb8f92de736c40b983d3897257d661812`。
日志、source delta、binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-device-gate-full-allocation\round2`。

其它 owner/marker/private headers、原 opaque Error/Anyhow/context、SDK/发现输出、
框架IO/task/allocator与完整freshness继续原范围审计。128MiB是已准入已知对象池，
非完整manager heap/RSS。L02未关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部
排除保持。仅本地提交，无真实设备、push或发布。
