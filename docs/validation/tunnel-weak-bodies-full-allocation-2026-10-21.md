# L02 single/dual pool strong-weak full allocation · 2026-10-21

原 shared_body/shared_dual_body 将 data Arc 与共享费用 guard Arc 分开；payload
已付费，但两处私有控制块未计量。本项改为应用声明的单个 Allocation<T,F>，
包含 strong/weak counter、原 T 和原费用 bundle。实际 Layout 包括所有对齐与
padding，单池或双池整份原 grant 在唯一 backing 前校验；没有 std Arc ABI 大小
估算、新池、费用副本或独立 guard allocation。费用字段位于高对齐 value 之前。
生产 manager/Input/RetainedRoute/Source receipt、publication Life、freshness
Metadata 已经通过原 wrapper 接线使用这个真实分配，非未使用的工具函数。

原字段在最后 strong 就地析构，支持 !Unpin；隐式 weak 在整个原析构（包括
panic 展开）期间保护 allocation 和费用。最后一个 explicit/implicit Weak 释放
实际整个 body 后，才归还原单池或双池费用。Weak upgrade 只对 strong>0 做
Acquire CAS，不能复活已析构值。弱引用路径只借用仍存活的 atomic counter字段，
不会通过已结束的 T 构造整个 allocation 引用。Send/Sync 与值/费用的线程安全
条件一致；不导出 raw pointer、任意 mutation 或 pin 接口。

测试夹具的独占修改原接口保留：对 implicit-only weak count 加锁后再检查
strong1，Release 解锁与 downgrade Acquire 配对。独立读取两个计数会与另一
strong 的 downgrade/drop 竞争，本实现使用原子锁和展开 guard。已查本机安装的
Rust alloc/src/sync.rs 对应同步顺序，未读取其私有 layout 作为预算。换池仍须
独占/无Weak且先取得整份新 grant，保持原值地址，旧信用到旧 token 析构才返还。

新增4项必要原语测试覆盖：双池先准入的真实单次高对齐 backing、!Unpin 原值
原地址就地析构、强引用结束后 Weak 保留原分配和两池费用、实际 dealloc hook
在任何 body 信用归还之前发生；无 explicit Weak 和有 Weak 两种原析构 panic；
确定性 downgrade/独占检查竞争及 observer 展开后解锁；8线程反复 upgrade 与
最后 strong/weak 竞争、原值地址/单次析构和不能复活。保留旧 single6/dual2 的
原 field buffer、field fee lastStrong、body fee lastWeak、128对齐、短/长/异池
拒绝、原值一次Drop/展开/零尺寸等断言。无效 grant 在任何 backing 前拒绝。

weak core4、strict CLI all-features/all-targets Clippy -D warnings、single6、
dual2、candidate8/intent7/receipt prepaid2、error payload9、完整CLI1746PASS/
0FAIL/13原ignored、build/fmt/diff及
实际只读manager1通过；Source/usbmux trap0。host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。Core/Cargo未改，既有Core证据复用，不声称重跑。
本轮没有执行 Miri、其它平台或设备验证。

首轮编译要求显式表达费用字段的独占借用，已按既有唯一性条件修正，无告警抑制；
同时按本机标准库核对 downgrade 的 Acquire 同步。第二轮前5门禁有成功终态，
原会话后续消失，完整CLI仅编译日志无 result，不计PASS。进程检查确认无在途
Cargo/rustc/python/test，683源码无漂移后仅串行补跑缺失6项，保留中断日志。
补跑完整CLI1743PASS/3FAIL/13原ignored：旧三处观察器只记录原字段/原胶囊，
现在还能看到受控的完整 root。保留原 UID/host/outcome/receipt 字符串地址、
尺寸、容量、对齐、先付费借记、短额度零复制和原错误/最后退款断言，新增恰好
一处最终 full-root backing 的真实尺寸与原值落在对应范围内检查，没有忽略分配
或允许额外 key 副本。生产分配算法不因此改变，仅同步一处旧 Life 注释。
第三轮增加三个定向观察器门禁并重跑全部14项，通过后提交。683冻结源码无漂移，
11处源码差异；binary SHA与实际host报告一致，受测文件已归档：
`7bbfac1c9621bc634945c1731e59b7f4bf179a49416abc958da445dd25c271ff`。
日志、source delta、binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-weak-bodies-full-allocation\round3`。

其它 metadata lease、marker/private headers、原 opaque Error/Anyhow/context、
SDK/发现输出、框架IO/task/allocator与完整freshness继续原范围审计。128MiB是
已准入已知对象池，非完整manager heap/RSS。L02未关闭，23交付/6缺开发/L02待
全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备、push或发布。
