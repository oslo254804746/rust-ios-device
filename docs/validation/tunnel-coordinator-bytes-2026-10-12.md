# L02 coordinator key/gate/backing 字节归属 · 2026-10-12

补记：[默认fixture隔离更正](tunnel-fixture-isolation-2026-10-12.md)发现本项初次全组
Rust门禁包含通用HTTP refresh的NativeSource路径。下列PASS是当时实际方法结果，
不能据此认证全部Rust测试没有真实发现；修正后的最新隔离复验另行登记。原CLI
readonly权限/trap证据仍独立有效，原日志和源码hash保留。

从交接 HEAD `cba082f6e1441253a14f5fbd6ac859e013716a8b` 继续，保留当前
`codex/review-fixes-20261010` checkout。初始工作树干净，交接列出的源码 hash 与
`target/debug/ios.exe` 的 SHA256 均匹配。完整532文件二进制来源清单复核时已开始
本项编辑，差异仅本项的 `tunnel.rs` 与 `lifecycle.rs`，没有未知来源差异。

## 生产实现

协调器使用一个有界 Vec 保存原 key 与 canonical gate，保留4096 namespace、
原强引用/持久状态 pruning、per-device serial 和全局/设备/intent epoch规则。
用实际 `Vec.capacity * size_of(row)` 收取共享128MiB额度，替代不暴露 bucket/control
分配字节的 HashMap backing。扩容时新旧 backing 同时收费；先准入新 backing 才
移动原 rows，拒绝时原 Vec、gate 和 ticket身份不变，没有网络等待或新建后台任务。

每个新 key 按原 owned String.capacity 与 gate inline size收费；生产构造器传入同一
raw snapshot/publication共享 bank。相同 key的后续 ticket克隆共用原 gate与charge，
不能重复收费。clear/stop仅使原 ticket失效，不等于释放存储。table销毁先释放
Vec backing，原ticket/publication/receipt仍持gate时保留原charge；最后一个gate owner
释放才归还。key可能先释放，仍保留其原准入charge直到gate最后释放，属于保守
所有权额度，不声称实时RSS或allocator已分配字节。

## 门禁与边界

新增4项验证精确共享额度、相同gate不重复收费、clear后的原ticket归属、表/最后
owner两种释放阶段、backing准入失败回滚、扩容拒绝保留原容量/身份，以及多次
扩容按实际capacity收费并释放旧 backing。受影响既有fixture现在将协调器接入
同一缩小bank，单独保留其准确charge；原publication/raw剩余额度、零余量、原native
joins、strict peer EOF和task baseline断言继续。初轮编译发现旧测试一处容器访问
遗漏，修正后新4项通过；失败日志保留。

完整受影响tunnel串行门禁373通过、0失败、2原ignored；含饱和4096 key、A/B本地
隔离、原error/epoch/native joins/strict EOF门禁。两个ignored仍为实际browser客户端
与显式L04测量，不改成PASS。严格CLI Clippy、CLI build及readonly manager实际fixture
1项均通过；后者仍以list/manage_server权限让生产Source发现保持零连接，独立trap
断言保留。主机runner为5 PASS、0 FAIL、22 NOT_RUN，real_device_executed=false。
fmt/diff通过。精确来源/二进制/log/commit由外部summary登记；未执行平台和设备
保持NOT_RUN，未变SDK/core协议门禁按影响复用，不称全量Rust/SDK重跑。

本项只增加协调器key/gate inline/Vec backing归属。manual desired/route的独立String
及snapshot克隆、watch/Arc内部heap、candidate/selection backing、retry/failure/scan/native
history与原error payload仍有未交付部分。原dyn Error/anyhow/native join错误不能用
inline size、opaque计数或采样代替创建来源合同和完整字节准入。不丢弃或格式化
替换原对象，不借本项关闭L02，完整freshness/reuse/restart/manual epoch核查继续。

账本保持23开发交付、6缺开发、L02待完整收口；有效30，Q04及全部子项排除。
无子agent、并行Cargo、推送或设备操作。每项验证后立即本地提交并自动推进。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-coordinator`。
