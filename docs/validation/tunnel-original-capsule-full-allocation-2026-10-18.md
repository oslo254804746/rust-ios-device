# L02 original Source capsule full allocation · 2026-10-18

原自动重试 Source 的 Original 胶囊包含 OnceLock<anyhow::Error> 和 body fee，
经私有 std Arc 共享。payload 已预付并跟随 first/latest/history，但私有控制块
未计量。本项复用已验收的 shared_allocation 精确分配，将 Original 原值与原 grant
放入同一个应用声明的 counter/value/grant layout，完整布局在 Source 之前同池准入。
不再为这一 Original 胶囊创建 std Arc；未改动已验证的共享分配原语。

Original 保留原 OnceLock，fee 在实际 allocation 内且在 value 外。reserve 先取得
完整 original root grant 和其它原胶囊 grant，再复制 selector/创建原 root；原错误
到达后仍填入这同一个 OnceLock，History.first/current/latest 仅共享原 root。没有
复制/重新封装原 anyhow/std IO，没有后置配额申请、新池、新计时器或历史替换。
原 primary/source/downcast/错误分类与 Ready poll 保存规则保持。最后一个 root
释放实际分配后才析构原错误与 grant。initial_bytes 测试合同计入完整实际布局。

保留原两项测试：8线程 history 克隆保留原 typed IO 指针、原析构先于费用退款且
只一次；first history 与 latest 两个独立原 IO/指针/退款边界。新增实际 Ledger
测试记录真实 backing：原胶囊值在唯一 observed allocation 范围内（不猜 offset），
完整 layout 与原 grant 相等且同池。其余尚未创建的两个 Removed owner 和 key/route
按实际布局计量，Ledger::start 完成后、Source 前池为零。关闭原池后捕获原 IO，
不创建新的 owned backing；primary/latest、原 capsule 和 IO 指针/Disconnected
类别不变；Ledger、结果和 coordinator 释放后，独立 receipt 仍持有原对象直到最终
全额退款。原 std/anyhow 外层包装分配仍是独立边界，此测试不声称计量该包装。

原 pre-paid selector 测试也保留：少1字节时零分配、每次 backing 前所需 grants 已
全部取得、两个字符串实际大小/原指针和最终退款。新原胶囊 backing 现在对观察器
可见，测试新增第三项完整 root layout 与独立指针检查，没有忽略或删除额外分配。

原胶囊3、reconnect21、错误归因9、完整CLI1737PASS/0FAIL/13原ignored、严格CLI
all-features/all-targets Clippy -D warnings、build/fmt/diff通过。实际只读manager1
通过，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
Core/Cargo未改；既有Core证据复用，不声称重跑。没有真实设备操作。

前三轮测试失败日志保留：新增测试误把后续 episode 尚未准入的590字节当成已用；
改为依据实际两个 Removed layout 与 key/route 计算，不使用字面常量，并保留完整
Source前零配额断言。新测试误写 ConnectionReset 类别 Io，现按既有 classification
映射记录捕获前的 Disconnected 并比较捕获后类别；生产分类未改。旧 selector 观察
器只预期两个字符串，现完整验证新胶囊为第三项。第四轮全部10个串行门禁通过，
681个冻结源码无漂移，3处源码差异。CLI SHA-256：
`4b42f21bf8f99afba61be79a6f73b3fb07be2f321af68d598b29696fbfab0f9e`。
日志、source delta、受测binary、current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-original-capsule-full-allocation\round4`。

其它 Receipt/owner 的 private header、原 stdIO/Anyhow/context 或 opaque allocation、
SDK/发现输出、框架IO/task/allocator与完整freshness继续原范围审计。128MiB是已准入
已知对象池，非完整manager heap/RSS。L02未关闭，23交付/6缺开发/L02待全审计、有效30
与Q04全部排除保持；仅本地提交，无push或发布。
