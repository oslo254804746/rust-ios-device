# L02 coordinator DeviceGate final owners · 2026-10-18

DeviceGate 原 body/key grant 位于 raw Arc payload 内。完整私有根审计仅有
DeviceTable、DeviceRow 和 Ticket，未发现 Weak、pin 或 raw Arc 出口；本项
统一为封闭 Shared<DeviceGate>。所有克隆消费原 Arc，原 allocation 先结束，
再析构 gate/原 key/body grant。原实际 sizeof<DeviceGate>、canonical key
准入位置及 Vec backing 收费保持，无新 sidecar、额度 padding 或 header 估计。

Shared 的借用 ptr_eq/strong_count 显示原 data Arc 身份和强引用数量，原
剪枝条件仍为 count > 1 或 persistent，4096 槽上限不变。Ticket 的 epoch/
version 比较、gate 更新、串行借用、desired listing、manual success、取消/
stop/clear 规则保持；原 generation/native/IO 的 Arc 比较未改。计数和地址
接口不提供 raw Arc、Weak 或 pin 出口。

新增2项通过：同 gate 的旧 tickets 保持原 key/root 地址，闭池/零余量下
table backing 先退，8线程最后 tickets 结束后准确退 body/key；真实4096槽
满表只剪除空闲 Automatic，活跃 ticket 和 table-only Stopped 保持原根，
clear 后旧 ticket 失效，新 ticket 复用原 gate、新 incarnation，最后全额
退款。原精确/差一字节、增长回滚、desired 副本及协调器取消断言均保留。

Gate专项2、lifecycle全组21、完整 tunnel737 PASS / 0 FAIL / 2原ignored、
upstream access10通过。all-features/all-targets CLI Clippy -D warnings、
build/fmt/diff、实际只读 manager1通过；Source/usbmux trap0，host5 PASS /
0 FAIL / 22 NOT_RUN，real_device_executed=false。

669个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`ed73d91eb5d376757bd48915d0d318ecda42c9c007f989a02799dee34d6f9513`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-coordinator-gate-owner\round1`。

共享 metadata permit 等 owner、terminal/anyhow/private header/opaque error/
IO、发现/SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。
128MiB为已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、
有效30与Q04全部排除保持。仅本地提交，无真实设备、push或发布。
