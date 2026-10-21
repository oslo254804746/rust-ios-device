# L02 freshness output refusal and cause recovery · 2026-10-21

本项补齐当前生产刷新路径的三个缺失验证，仅修改既有 native freshness 测试
文件，没有新建替代生产路径。所有目录仍读真实localhost H2/XPC，原生生命周期
仍由既有真实userspace worker夹具验证；仅metadata/routing使用原cfgtest seam。

第一项在Source成功取得第二份完整目录之后，按新目录的actual known_bytes加
完整MetadataOwner body精确留下publication N−1。旧目录仍在cache/table/registry
与独立观察owner中付费，新目录必须与旧目录同时存活才能交换；提前退旧费会
错误地允许新目录通过。拒绝后断言原key、NativeLease、publication、两类lease、
runtime、old catalogue及RefreshProof地址/身份保持，原global临时目录/body
准确退款，publication仍恰好N−1。Source成功的输出拒绝不伪装为Source failure，
不创建registry first/latest cause或新native lease。释放fixture占用和实际清理
完成后，两池准确退满。

第二项逐一发送wrong UID、缺UniqueDeviceID、缺Services和缺Port的真实H2目录。
wrong UID先形成完整catalogue，manager原Protocol检查拒绝；缺失required字段
由Core checked握手以Final协议错误拒绝，未形成partial catalogue。原cache/
table/registry/runtime/leases和publication费用不变。错误路径保存原CoreError
地址，first/latest指向同一原Receipt，原bank和body/report费用准确保留；
Source成功后wrong UID不产生Source失败胶囊。旧目录继续可读，最终实际joins、
严格EOF和完整退款保持。

第三项连续两次独立IO失败，分别记录原std::io::Error和其内部Original payload
地址，保留ConnectionReset、PermissionDenied、source委托和marker身份。两个
registry first/latest Receipt根独立、费用属于原池；成功刷新后registry清空
两类cause，cache/table/registry切到新目录而key/runtime/lease不变。调用方原
两个report仍保留原IO地址和各自完整Receipt/body与两份report保留费，逐一
最后释放只退各自费用；不重建或格式化替换原错误。opaque anyhow外壳和任意
原error内部allocation仍是原未完成物理边界，本项不据sizeof宣称完整计费。

测试helper只扩展原producer接受真实目录字典、Source成功Ready观察hook、
独立IO脚本及原地址记录。OriginalRefresh通过原NativeLease::capture_refresh
取得registry proof，同时保留原roots；原H2 bootstrap/frame/EOF、已有14项
freshness测试、原typed IO/Work指针与native joins断言均保留。没有扩大固定
quota、忽略或删除测试。子Agent开发及静态自审，主控复核、格式化、冻结并
串行执行所有Cargo及cargo build。

8个验收命令通过：native freshness17（原14+新增3）、strict CLI all-features/
all-targets Clippy -D warnings、完整CLI1756 PASS / 0 FAIL / 13原ignored、
build/fmt/diff、实际只读manager1（Source/usbmux trap0）、host5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。Core/Cargo与生产代码未改，复用55f21fe
的prepared Core8与owned真实service restart证据，不声称本轮重跑Core测试。
684冻结源码无漂移，唯一源码差异为本项CLI测试文件；归档CLI与实际host报告
SHA-256一致：`7b85dda5c1ac7ebb5f29633edf19e3960d2d6c1548bafe38f3c0b5b83ff7aaa1`。
日志、source delta、verified-ios.exe、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-freshness-refusal-recovery\round1`。

RSD endpoint本身重新发现/staged替换仍待实现；目录service port变化与实际
service restart不能代替endpoint迁移。其它marker、publication实际Box最后
费用、opaque Error/Anyhow/context、SDK/发现输出、框架IO/task/allocator等原
L02范围继续。128MiB为已准入已知对象池，非完整heap/RSS。L02不关闭；主项
23交付/6缺开发/L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实
设备、push或发布。
