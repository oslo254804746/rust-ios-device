# L02 native report provenance identity allocation · 2026-10-21

Core Ledger 与 TunnelShutdownFailure 原先共享私有 Arc<()> 身份标记，未对其
实际控制块计费。报告可以晚于 Ledger 结束，因此该费用不能随 Ledger 值结束。
本项声明完整强引用根 Allocation：AtomicUsize 与原 Option<OwnedSemaphorePermit>。
无 Weak、raw owning pointer、pin 或可变入口，不估计标准 Arc 私有 ABI。

metadata-backed Ledger 先从原 bank 一次支付原 Ledger 值与完整身份根费用，
在任一 backing 前完成；随后分割身份的精确原 grant，校验 bank/完整布局，
创建真实容量一根，最后创建原 Ledger。剩余 Ledger 费用贯穿所有 fallible
准备，不是晚期 top-up 或固定 publication 额度。无 bank 的旧构造继续其
unmetered 分支，不声称该分支已计费。

原 native IO、JoinError、Elapsed、cleanup 数组、实际 join、retry snapshot
和同 Ledger work-report 识别逻辑保持。报告与 Ledger 克隆同一个身份根，
不保留 Ledger 本身，避免 provenance/cause 引用环。最后强引用 Release/
Acquire 后，先 dealloc 同一完整根，再释放原费用；报告仍活着时身份费仍在
原池。身份字段宽度与公开 report 接口保持，不重建或字符串化原错误。

新增两项测试验证 foreign/短/长整笔 grant 零 backing、首个 backing 前整笔
扣费、实际布局/地址、闭池零额度克隆、真实 after-free 信用点及八线程最后
owner 精确退款。旧 Ledger N-1/N 测试增加整份 Ledger+身份费用先于首个
identity 分配的观察点。原十个独立 IO 与 attempt Elapsed 测试保留所有
指针、cleanup 与 joins 断言，并闭池生成两份报告：Ledger 结束只退其值费用，
第一份报告结束不退身份费，最后实际身份根释放后才退身份费。

第一轮 owner、无 mDNS owner、XPC、existing、prepared、两套严格 Clippy、
activation 与 proxy 通过；完整 tunnel 131 PASS / 2 FAIL。两处旧测试漏掉
新身份费用或在原错误报告仍活着时期待全额退款，差额均为此 host 实际根
24 字节。第二轮仅使 derived reservation helper 在 crate 内可见，修正
cfgtest runtime owner 公式并显式验证 socket 错误报告结束前/后的原费用；
未扩大固定额度，原 typed fatal、严格两端 Ok(0) EOF、实际 joins 与 task 数
断言全部保留，第一轮日志保留。

第二轮 21 个命令全部通过：owner 25、无 mDNS owner
20、完整 tunnel 133、XPC 216、
existing 22、prepared 13、activation
9、proxy 4，无 mDNS current
6、initial 18、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有既有 dead-code warning，不称作严格无警告。全部 Cargo/build
由主控串行执行。

686 冻结源码无漂移，五处源码差异；Core 生产源码已改，本轮使用实际 Core
和 CLI 结果。实际 host 与归档受测 CLI SHA-256 一致：
df1e2ac07bd829ba3af7f6733ce2deb3fe09f72a529080477261257c025e3daf
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-report-identity\round2
未声称 Miri、其它平台或真实设备验证。

Ledger 自身标准 Arc 控制块与其 body 费用最终释放、child set/watch/stopped
共享根、opaque 原 report/cause/wrapper、其它 marker、SDK/发现及 metadata
副本、HTTP/WS/配置/凭据、框架 task/allocator 与完整 endpoint 恢复均继续
原范围。本项只收口独立身份分配。128MiB 是已准入已知对象池，非完整
heap/RSS。L02 未关闭；23交付/6缺开发/L02待全审计、有效30与Q04全部排除
不变。仅本地提交，无真实设备、push 或发布。
