# L02 service route projection from original storage · 2026-10-21

普通 connect_rsd_service 与 connect_xpc_service 原先先复制完整名称和 feature
Vec，再仅使用 port/shim 决策或丢弃 metadata。这些副本没有原目录费用归属。
本项让同一私有 resolver 在原目录仍活着时同步投影：raw 路径只返回 port 与
shim bool，普通 XPC 路径只返回通过原 shim 检查的 port。名称与 features
直接借用原目录，不产生这些临时副本，也不增加 metadata 准入。

current prepared、cached owned initial、on-demand owned initial、cached legacy、
on-demand legacy 的原优先顺序、五次 retry/backoff、ResourceLimit final、
原最终 cause 和 initial observation 保持。原一次 lookup、一次 service dial
不变。prepared 选择原 row；owned initial service view 保留同一原根；新发现
目录也在投影完成后才结束。回调输出不能借用该临时目录。legacy shim 查找
改为原 key 字节与 requested + 固定 suffix 的 iterator 比较，去掉 format
临时 String，canonical 仍优先。

raw service 的原 RSDCheckin 条件与交换保持。普通 XPC 仍先拒绝 shim 再拨号。
两种 XPC 入口共用原 connect_stream、endpoint 与 reconnect 顺序，无新的
Source、bank、任务或期限。公开 connect_xpc_service_with_metadata 继续复制
并返回原 String/Vec 输出形状与内容，其出口副本费用仍属原范围待处理；本项
没有将整个 SDK、service stream 或 XPC 框架认定为零分配或完整计费。

新增两项指针测试：legacy canonical/shim 投影恰调用一次，名称与 feature
地址来自原 descriptor；prepared owned 在闭池零额度下仍选择 canonical 或
shim 原 row，名称/feature 地址不变，缺失服务不运行回调，不产生受控 backing，
最后原 owner 精确退款。现有 current 同/变端口实际服务重启、严格 EOF/join、
initial shim、公开 metadata、真实 retry 与原初始 IO 保留测试继续通过。

第一轮编译发现 owned service 尾表达式临时 iterator 的析构顺序不满足借用
要求，Cargo101 后终止；第二轮将 iterator 绑定局部，使其先于 service 析构。
没有修改路由、重试或错误优先级，第一轮日志保留。第二轮 21 个命令通过：
service routing 12、connected 8、
prepared 13、existing 22、XPC 216、
activation 9、proxy 4、tunnel
131；无 mDNS current 6、
initial 18、proxy 5；
Core/CLI all-features/all-targets Clippy -D warnings、完整 CLI
1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、实际只读 manager 1
（Source/usbmux trap0）、host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。no-default 与 tunnel-only check 通过，保留既有
dead-code warning，不称作严格无警告。全部 Cargo/build 由主控串行执行。

Core 生产源码有改动，本轮使用实际 Core 与 CLI 结果。685 冻结源码无漂移，
四处源码差异；实际 host 与归档 CLI SHA-256 一致：
4719f55ac155e28f66f3346cca88075957a8a1df2d65e47635ab04c3c3b86bb8
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-service-route-projection\round2
未声称 Miri、其它平台或真实设备验证。

公开 metadata/SDK/发现副本、legacy raw Arc/Weak、opaque 错误、其它 marker、
HTTP/WS/配置/凭据、框架 task/allocator 和完整 endpoint 恢复继续原范围。
128MiB 为已准入已知对象池，非完整 heap/RSS。L02 未关闭，23交付/6缺开发/
L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
