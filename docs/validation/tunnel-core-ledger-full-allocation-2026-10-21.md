# L02 Core native Ledger full allocation · 2026-10-21

5415fc6 已独立支付跨报告的 provenance 根；Ledger 自身仍是标准 Arc，其
私有控制块未完整计费，值内费用也不能证明实际根已结束。本项将 crate 内
Ledger 共享接口改为封闭强引用 handle，Deref 到原 LedgerState。原状态字段、
outcomes、pending/join 计数、record/snapshot 方法与 IO/JoinError/Elapsed
cause 对象保持；原 Arc clone 接线替换为 handle clone，公开 SDK 接口不变。

实际声明 Allocation 包含 AtomicUsize 与 ManuallyDrop<LedgerState>。
State 原 body grant 支付整个实际根，不猜标准 Arc ABI、不增设费用 guard，
不导出 Weak、raw owning pointer、pin 或可变引用。克隆原强引用，最后
Release/Acquire owner 移出已知 Unpin State，先 dealloc 同一容量一根，再
析构 State、原 cause 和原费用。Send/Sync 明确要求原 State Send+Sync。

metadata 构造一次从原 bank 支付完整 Ledger 根与独立 provenance 根。先
分割身份费用，实际空 Ledger 目的地在身份的 fallible 分配期间持有整笔
body fee；目的地字段按 Vec、after-free 测试哨兵、fee 排列，任何失败先
结束实际空根再退 body fee。全部 fallible 步骤完成后才移动原 body fee
并原位填 State，无晚期分配、top-up、零 grant 占位或新 bank。无 bank 的
旧构造保持 unmetered 语义，不声称 kernel/no-bank 路径有了全局准入。

报告只持有既有独立 identity，不持有 Ledger，保持原无 cause cycle 结构。
Ledger 实际根结束后 body 全费用可独立退款，旧报告继续保留原 IO 与
identity 费用，最后报告结束才退 identity。所有同 Ledger work-report
识别、重试报告、原 cause 指针、cleanup 与实际 join 逻辑保持。

新增四项根测试：foreign/短/长 whole body grant 零 backing；真实空根后
身份准备失败时 after-free 先于 body credit；闭池零额度的原根/State/
cause 地址及真实根释放先于原 cause 自定义析构、State body fee 仍在；
八线程最后根原地址与精确一次退款。既有 N-1/N 观察点验证 Ledger 根与
identity 根两次实际分配均在整笔扣费后；原十个独立 IO、attempt Elapsed、
跨报告身份及 joins 断言保留，增加实际 Ledger 根 after-free 前任何费用
均未退的观察点。只更新按完整布局派生的预算与 handle/State 公式，没有
扩大固定额度、删断言或新增忽略。

一轮 21 个验收命令通过：owner 29、无 mDNS owner
24、完整 tunnel 137、XPC 216、
existing 22、prepared 13、activation
9、proxy 4，无 mDNS current
6、initial 18、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有 dead-code warning，不称作严格无警告。所有 Cargo/build 均由
主控串行执行。

687 冻结源码无漂移，十五处源码差异，其中多数是 crate 内共享 handle
类型与 clone 接线。Core 生产源码已改，本轮使用实际 Core/CLI 结果。
实际 host 与归档受测 CLI SHA-256 一致：
101224f27fc9288119b0d3fb2416bc8fe019d43982c536a172938dfd04607320
证据、source delta、verified-ios.exe、current 与提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-core-ledger-full-allocation\round1
未声称 Miri、其它平台或真实设备验证。

child set/watch/stopped 共享根、公开 native handle Arc/Weak、opaque 原
report/cause/wrapper、其它 marker、SDK/发现及 metadata 副本、HTTP/WS/
配置/凭据、框架 task/allocator 与完整 endpoint 恢复继续原范围。128MiB
是已准入已知对象池，非完整 heap/RSS。L02 未关闭；23交付/6缺开发/L02待
全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
