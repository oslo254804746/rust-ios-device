# L02 Source-preflight receipt/write owner full allocations · 2026-10-19

刷新 Receipt 和失败写入 FailureWriteOwner 原 payload 已先付费，但共享 std Arc
控制块未计量。本项改用应用声明的完整共享分配；counter/value/原 grant/padding
的真实 layout 在 Source 前同池准入。body grant 在真实 allocation 内、value 外，
原报告额度、context 和动态数组仍分别由原费用持有，不猜 std Arc 私有大小。

刷新 Receipt 保留原 OnceLock<anyhow::Error> 和两个 report grant。prepare 一次
取得 whole root + 两份原 Failure payload 额度，拆出报告费用后在原 bank 创建
同一个空 root。原错误到达后 capture、report、first/latest/独立 snapshot 只借用
或共享该 root；原 IO/downcast/source/class、最多两份报告及最后退款不变。外层
Failure/Anyhow 私有包装仍是独立边界，没有据此宣称它们也已完整计量。

FailureWriteOwner 保留原 Mutex<WriteState>，完整 root 与 SourceFailuresContext
费用在 intents/receipt 数组和 Source 窗口之前取得。数组建好后用原 exact grant
创建 owner。原 receipts、in_scan/finished、finish/Drop/Session 取消、完成分类与
主/清理错误、whole clock、回滚、真实 native lease/joins 语义保持。已预付 KnownVec
backing 不重复算入固定 root。仅将 body/backing 预算 helper 改为实际完整 root。

增强原测试，保留原 source/IO/并发/退款断言：刷新0/最后1字节拒绝在 backing 前
完成，精确额度观察到一个实际 root 且原值落在正确 allocation 内；报告在关闭
原池后不创建替代 owned backing，原 IO/类别与两份报告退款按原顺序保持。写入
batch 容量0/1/4分别观察到恰好1或4处 backing，核对原 destination、intents、
receipt 数组真实布局与指针，以及最后 whole root 在分配前已全额付费；原8线程
只完成 Pending receipts、保留原已完成状态、原 Session/结果/最后 owner 寿命保持。
没有新增仅镜像实现的测试。

Receipt2、bookkeeping8、freshness21、error payload9、严格CLI all-features/
all-targets Clippy -D warnings、完整CLI1742PASS/0FAIL/13原ignored、build/fmt/
diff及实际只读manager1通过。Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。Core/Cargo未改，复用既有Core证据，不声称重跑。

首轮定向Receipt/strict/bookkeeping通过，freshness20PASS/1FAIL：旧精确准入
夹具仍使用 sizeofReceipt payload。按实际 whole root helper 更新该 fixture，
保留 minus1 拒绝、精确成功、全额退款和实际零额度原错误目标；生产代码未因此
改变，失败日志保留。第二轮全部11个串行门禁通过，681个冻结源码无漂移，6处
源码差异。受测binary已归档，SHA与实际host报告一致：
`3361a02e085a12a91a9e9ca34ebab7aebf20a35a39deb4e0bf7521f9de29ab5b`。
日志、source delta、binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-pre-source-owners-full-allocation\round2`。

其它 owner/marker/private headers、原 opaque Error/Anyhow/context、SDK/发现输出、
框架IO/task/allocator与完整freshness继续原范围审计。128MiB是已准入已知对象池，
非完整manager heap/RSS。L02未关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部
排除保持。仅本地提交，无真实设备、push或发布。
