# L02 native census current snapshot storage · 2026-10-18

原 payload_outcomes 在数组准入后，对每个新 shutdown_status 错误另外
Arc::new，wrapper payload 未单独准入。本项将 census 元素改为实际 inline
PayloadOutcome：History 保留同一原 Arc，Current 持有本次取得的完整原
TunnelError 值。两个实际 error/handle backing 在 clone/状态查询前准入，
实际 enum 槽位包含当前值，不在查询之后新增 CLI Arc wrapper。AsRef/Deref
只借用原 TunnelError，保留错误类别、typed source/downcast 和原 IO 指针。

旧历史与本次快照分别持有原证据，没有复用、改写旧报告。Core 的每次
TunnelShutdownFailure 是新的不可变 joins 快照，固定 retained cause 数量
不表示快照版本有限。required drain/capture、历史根、Elapsed/Cause Arc、
Core stdIO/private wrapper 和 terminal anyhow 是独立未完成边界，本项不
改变其语义，也不据本次 census 路径推导已解决全部 native report 分配。

新增实际 worker 测试确认当前 TunnelError 地址位于实际已付费数组区间内，
两份真实 backing 在 allocator hook 前均已收费；测得整笔差一字节拒绝时
部分数组准确退款，原 active owner/原 work 指针保持。精确准入后闭池零
余量，原 census pending-joins 快照经过实际 certified joins/task baseline
后仍冻结，原 cause 指针保持，最后数组只退自身实际费用。原 native quota
拒绝、strict EOF、最后 owner 和原历史同 Arc 断言保留。

Census专项3、retirement全组35、payload attribution9、完整 tunnel746 PASS /
0 FAIL / 2原ignored、upstream access10通过。all-features/all-targets CLI
Clippy -D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux
trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
round1测试编译有一处旧历史断言仍预期Arc返回值，已显式匹配History并保留
同一 Arc::ptr_eq，生产未变；round2全部重新验证，失败日志保留。

673个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`9b1c4ebca63452e7ef724878da4e08bef83e6b24d464ca6672fa65c63c07c706`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-census-inline\round2`。

其他 native report/owner、terminal/anyhow/private header/opaque error/IO、
发现/SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB
为已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30
与Q04全部排除保持。仅本地提交，无真实设备、push或发布。
