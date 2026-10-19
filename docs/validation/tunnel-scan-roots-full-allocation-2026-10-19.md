# L02 scan root full allocations and preallocated abandonment · 2026-10-19

扫描 Outcomes、不可变 HistorySnapshot、AbandonedScan 原共享对象按 payload
付费，std Arc 控制块未计量；弃置对象本身还在 Source 已返回错误后的 Drop 路径
创建。本项将三者换成应用声明的完整共享分配，counter/value/原 grant/padding
的真实 layout 由同一个原 bank 准入，没有 header 大小估算或 sidecar。

Outcomes 初始化创建完整实际 root。begin 的一次 prospective reservation 同时
包含原 compound/cleanup payload 和两个完整 child root，在 Source 前实际分配
空历史快照及空弃置 OnceLock<anyhow::Error>。只有两者成功后才将 slot 标为Active。
Source 返回/取消后的 Session Drop 只将同一个原 primary 填入该单元、发布原 root、
冻结原历史快照，不再申请配额或创建共享 backing。未使用的空 root 随正常结束
归还费用。原 take/finish/write-owner、primary/secondary/context、错误链和清理
规则保持；Debug/source 借用填入的同一 anyhow，没有错误克隆、额外层或格式化。
新 owner 补齐与旧 owner 相同的只读 AsRef 借用接口。

新增实际分配测试检查：Outcomes恰好一次完整 backing；begin 恰好两个 child
backing，完整费用在每次分配前已取得，原对象位于各自实际 allocation 范围内，
不猜 offset；弃置单元起初未填入。原 IO 在 bank/outcomes 关闭后留存，Drop 的
allocator hook 禁止任何新 owned backing，取得的 history/record 仍为预分配原
地址、原 IO downcast 指针和类别。Outcomes/快照释放后独立 record 仍持有原费用，
最后归还精确总数。原测试继续覆盖1MiB opaque原对象且Display会panic、八线程
历史持有者、冻结旧顺序、关闭池、最后析构/退款和原 cleanup/完成错误。

原 body 预算断言改为实际 whole-root helper，未删除指针、短额度、Source前拒绝、
原错误与冻结快照断言。新的 NonNull owner 会改变 Option/inline array 的实际
布局，费用始终从当前具体类型计算。SourceContext、FailureContext、终态包装和
外层 Anyhow 私有分配仍是独立待审计边界，不据此宣称错误后所有 heap 都无分配。

scan17、严格CLI all-features/all-targets Clippy -D warnings、error payload9、
完整CLI1742PASS/0FAIL/13原ignored、build/fmt/diff与实际只读manager1通过。
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
Core/Cargo未改，既有Core证据复用，不声称重跑。首轮编译发现旧报告调用方依赖
AsRef，补回等价只读接口后第二轮全部9个串行门禁通过，失败日志保留。
681个冻结源码无漂移，5处源码差异；受测binary已归档且其SHA与实际host报告一致：
`2f2066b2925a6d9b759efc452c3c4c1539b163c1861ce3f1b84f98cbc16791ed`。
日志、source delta、binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-scan-roots-full-allocation\round2`。

其它 owner/marker/private headers、原 opaque Error/Anyhow/context、SDK/发现输出、
框架IO/task/allocator与完整freshness继续原范围审计。128MiB是已准入已知对象池，
非完整manager heap/RSS。L02未关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部
排除保持。仅本地提交，无真实设备、push或发布。
