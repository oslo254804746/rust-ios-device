# L02 private runtime unit-stop root · 2026-10-22

原 userspace stop watch 使用不透明 Tokio Shared 分配，未包含在原已准入
Ledger 完整根中。本项将该私有 unit-stop 协议的 Mutex、版本、发送者计数和
Notify 直接放入同一 LedgerState。实际完整声明根布局在 listener bind 和
workers 前由原 metadata bank 准入；初始化通知命名空间、clone、subscribe
和 send 不创建另一个受控共享 backing。Sender/Receiver 保留同一原 Ledger，
State 内没有 owner 回指，不形成循环，也不新建 bank 或扩大固定额度。

只替换 userspace 四处丢弃 changed 结果的私有停止信号。公开 manager/native
TunnelHandle 与 forward 的 Tokio watch 类型保留。私有 Closed 不进入 SDK、
IO/source 错误链，不能称为保留 Tokio 原 RecvError 类型。原 cause 记录→
Release stopped bit→通知、reader/writer 的 biased IO 优先、smoltcp/listener
的 biased stop 优先、期限、原 native JoinError、abort/drain/实际 joins 保持。

依据 Cargo.lock 固定的 Tokio 1.50.0 源码逐条保持：每次 equal/unit send
与零接收者 send 均 wrapping +2；subscribe 当前值已 seen，receiver clone
复制旧 seen；changed 先交付最后 unseen version 再报告关闭，has_changed
先查关闭；取消未完成的 changed 不改 seen；创建 Notified 后再检查版本，
spurious wake 继续等待；最后 sender 关闭不推进版本，解锁后 wake-all。
changed 保留公开 cooperative 包裹，预算耗尽时不提前消费版本。Sender 先
clone 原 Ledger 再增加计数，原根计数上限约束计数；原 LedgerState Unpin
编译断言保留，未新增 unsafe。单个 inline Notify 不代表与旧分片通知结构
具有相同性能。主控核对 pinned primary hashes，静态只读复核未发现可操作
缺陷；该审查不代替运行验证。

新增10项测试覆盖与 pinned watch 的 equal/repeated/coalesced send、late
subscribe、旧/新 seen clone、零接收者后重新订阅、最后 sender seen/unseen
关闭、closed 优先级、注册后取消/已唤醒但其它 biased 分支胜出、32接收者
wake-all、cooperative 预算耗尽，以及跨线程注册/send、spurious wake、版本
回绕和并行 sender。实际根测试观察 N-1 零受控 backing、exact N 两处原
Ledger/identity 分配前扣费、inline State 落在原 allocation 地址范围、闭池
零额度 clone/subscribe/send 的同一根与 A/B 隔离；最后 pending receiver
future 取消后，先实际 Ledger backing dealloc，再精确退款。原 durable IO
测试保留首个通知后可见的原类型、原指针与 source 链；通知 receiver 作为
真实新保活 owner，须随 storage 结束后才断言 Ledger body 退款。

R1 首个严格 Core Clippy 在新增测试比较 Result<bool,RecvError> 时发现
RecvError 无 PartialEq，退出101；仅将该断言改为检查原成功布尔值，未改
生产逻辑或弱化预期。保留 R1 日志，重新格式化/冻结后 R2 全部29命令通过：
unit watch 10 / 无mDNS 10，完整
tunnel 159，owner 42 / 无mDNS
37，listener 3 / 无mDNS
3，channel 13 / 无mDNS
13，原 stop 2 / 无mDNS
2，XPC 216、existing
22、prepared 13、activation
9、proxy 4；无mDNS current
6、initial 18、proxy
5。Core/CLI all-features/all-targets Clippy -D warnings、
完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、实际只读 manager
1（Source/usbmux trap0）、host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。minimal 与 tunnel-only check 通过，仍有既有
dead-code warnings，不称为严格无警告。全部 Cargo/build 由主控串行执行。

690 冻结源码无漂移，五处源码差异；Core 生产代码已改，本项使用本轮实际
Core/CLI 测试。实际 host/归档二进制 SHA-256：
f13ce9b2c5aed22c7e84c40d5e4cfdebe4cba6f6190d3b62a8f310a02963161a
证据、source-delta、verified-ios.exe、current/提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-runtime-unit-watch\round2

Notify waiter/Waker、框架任务/allocator、公开 native Arc/Weak、opaque
report/cause/wrapper、global/device/intent/retry marker、SDK/发现及公开
metadata 副本、HTTP/WS/配置/凭据、完整各模式 endpoint 恢复继续原范围。
128MiB 为已准入已知对象池，非整个 heap/RSS。未声称 Miri、其它平台或真实
设备验证。L02 未关闭；主项23交付/6缺开发/L02待全审计、有效30与Q04全部
排除不变。仅本地提交，无真实设备、push或发布。
