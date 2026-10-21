# L02 Core child task set full allocation · 2026-10-21

原 child task set 的标准 Arc 私有控制块未完整计费，值内 body 费用也不能
证明实际分配已结束。本项以封闭强引用 handle 共享原 ChildState，并声明
整个 Allocation：AtomicUsize、ManuallyDrop<ChildState>、原可选费用槽。
原 bank whole grant 支付该完整实际布局，不估计标准 Arc ABI，不增设堆
费用 guard。无 Weak、raw owning pointer、pin 或可变 owner 入口。

ChildState 保留原 JoinSet mutex、Ledger、cleanup slot、abort 标记和原
JoinError 保存槽。原 lock、drain、ChildGuard、abort/reap/record_joined
逻辑保持；只将 crate 内 Arc handle/clone 接线改为新共享 handle。
metadata 构造先用原 bank 精确支付并分配完整空根，在目的地持有 backing
和 grant 时构造原 State，随后原位填根；无 bank 的旧构造保留 unmetered
语义，不声称 kernel/no-bank 入口已计费。

最后强引用 Release/Acquire 后，只移动费用到栈上的 Release guard，
原 State 始终在原地址析构。State 正常结束或 unwind 后，guard dealloc
同一个容量一完整根，然后才释放原费用。没有移动异步集合状态，也不要求
State Unpin；没有用 State Drop 当作实际 free。原生任务仍由原显式 drain/
join 流程收集，最后 owner 析构不被当作新 join 证明。

新增三项根测试：foreign/短/长整笔 grant 和 N-1 零 backing 拒绝、exact N
实际分配前 whole debit；闭池零额度原 root/State 地址、原位 State 析构、
所有 State 字段结束后才真正 dealloc、fee 晚于实际 free，覆盖普通结束与
typed State 析构 panic 的原 payload 地址和展开；八线程最后 clone 原地址
与一次完整退款。测试 State Drop/after-free hook 仅供观察，生产状态析构
路径没有测试 payload。原 child-set 实际 spawn/drain 测试强化为闭池执行，
保留 pending 1→0，保存的原取消 JoinError 指针跨 clone 不变，最后实际根
结束后全额退款。原 panic/foreign-abort、cleanup、严格 EOF 与 joins 断言
继续通过。预算只替换为实际根布局派生公式，没有扩大固定额度或新增忽略。

第一轮 owner32、无 mDNS owner27 与相关 RSD 测试通过，严格 Core Clippy
发现新测试的 err/ok + expect 写法而终止。进程终止后仅改为直接匹配结果，
未改生产代码、加 lint 豁免或删断言，第一轮日志保留。第二轮 21 个命令
全部通过：owner 32、无 mDNS owner
27、完整 tunnel 140、XPC 216、
existing 22、prepared 13、activation
9、proxy 4；无 mDNS current
6、initial 18、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有既有 dead-code warning，不称作严格无警告。全部 Cargo/build
由主控串行执行。

688 冻结源码无漂移，六处源码差异；Core 生产源码已改，使用本轮实际
Core/CLI 结果。host 受测与归档 CLI SHA-256 一致：
92d0bc9e52a2d21d8de931d3196940ad62625cc332c3418cca6599eb1df7ffdc
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-core-child-full-allocation\round2
未声称 Miri、其它平台或真实设备验证。

watch/stopped 共享根、公开 native handle Arc/Weak、opaque 原 report/cause/
wrapper、其它 marker、SDK/发现及 metadata 副本、HTTP/WS/配置/凭据、框架
task/allocator 和完整 endpoint 恢复继续原范围。128MiB 为已准入已知对象
池，非完整 heap/RSS。L02 未关闭，23交付/6缺开发/L02待全审计、有效30
与Q04全部排除不变。仅本地提交，无真实设备、push 或发布。
