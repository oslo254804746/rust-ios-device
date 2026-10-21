# L02 channel and native slot complete roots · 2026-10-21

原 metered channel 使用标准 Arc<Shared<T>> 与 Arc<Semaphore>，费用只覆盖
两个值本体，私有控制块及最后实际分配结束尚无完整证明。本项声明两份
完整 Root 布局，各含 AtomicUsize、原值与原 grant 槽；不猜标准 Arc ABI，
不新增堆费用 guard。Root clone/Deref 封闭，无 Weak、raw owning pointer、
pin 或可变 owner 入口。任意 T 不要求 Unpin；最后 Release/Acquire 只移动
费用到栈 guard，原位析构 T，正常结束或 unwind 后真正 dealloc 同一容量一
根，再释放其独立原费用。

原 Tokio Semaphore 放在自己的稳定 Root 中。Slots 的 acquire_owned future
拥有原根，继续调用原 Semaphore.acquire/try_acquire。私有 retain 只对从
同一根获取的原 borrowed permit 延长内部寿命，并先 clone 同一根保活。
SlotPermit 字段按原 native permit、原 Slots root 排列，原 permit 先归还，
root 后结束。没有导出 native permit、forget 或 raw 接口，不移动 Semaphore。
许可可越过 channel shared 根寿命，自己的完整根费用仍持有到实际结束；
无 slot→channel 强引用环。

锁定 Tokio 1.50.0 的 borrowed/owned acquire 均调用同一 ll_sem.acquire(1)，
try-acquire 也调用同一原低层方法。原公平等待、取消、close、typed error
及 admission 后无 await 的 enqueue 保持；没有改成新的容量/通知算法。
本地 primary 源码摘录与 SHA 已归档，未通过记忆猜测私有布局。

with_bank 在任何 backing 前一次从原 bank 支付 shared 根、slot 根和
Entry<T> ring 的完整布局，分割原精确 grant，真实预分配三个目的地。
ring 使用 exact Vec，再经标准库公开保证 O(1)、不重新分配且不额外分配的
From<Vec> 转成 VecDeque，保留原 pointer/capacity。全部 fallible 准备完成
后才填两个原根，没有新 bank、晚期 top-up 或容量估计尾款。ring、slot 根
与 shared 根的独立费用分别保留；receiver 仍在原 mutex 外析构消息及 ring。
无 bank 分支继续原 Tokio channel。

保留原七项 channel 测试，包括阻塞 producer FIFO、取消首 waiter 的位置、
receiver close 返回原 Box、send/recv 取消唤醒、原 OwnedBuffer 最后费用、
receiver 外锁销毁与构造拒绝时无 worker/严格 EOF。旧 body 预算只改为两
份实际根布局，不扩大固定额度。新增六项验证 foreign/短/长 grant 零 backing，
whole N-1 在三个 backing 前拒绝、exact N 首次分配前整笔扣费、两根/ring
实际 layout 与地址；闭池原指针和各实际 after-free 信用点；原 slot permit
独立越过 channel 根、原 native permit 归还和 typed close/NoPermits；非
Unpin 原值原位析构及原 panic 展开；八线程最后根一次退款。

第一轮严格 Core Clippy 编译新测试观察器时，局部变量遮蔽回调，进程终止。
第二轮仅改测试辅助变量名，生产逻辑、断言与 lint 规则保持，第一轮日志
保留。第二轮 25 个命令通过：channel 13、无 mDNS channel
13、完整 tunnel 148、owner
32、无 mDNS owner 27、stop
2、无 mDNS stop 2、
XPC 216、existing 22、prepared 13、
activation 9、proxy 4，无 mDNS current
6、initial 18、
proxy 5；Core/CLI all-features/all-targets Clippy
-D warnings、完整 CLI 1765 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、
实际只读 manager 1（Source/usbmux trap0）、host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。no-default 与 tunnel-only check
通过，仍有既有 dead-code warning，不称作严格无警告。

主控执行全部 Cargo/build，另由子 Agent 只读复核 private permit 原根保活、
字段析构、任意 T 原位 drop/unwind、whole grant 与原 Tokio 语义，未发现
可操作缺陷；审查未修改源码或运行 Cargo，范围限静态源码，不替代门禁。
689 冻结源码无漂移，三处源码差异。Core 生产代码已改，本轮使用实际
Core/CLI 结果；实际 host 与归档受测 CLI SHA-256 一致：
a5691b1746ee2c8420484c8b9b68184abc4cebb739939513ae3604bd58587a9e
证据、static review、source delta、verified-ios.exe、current 与提交后
rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-channel-full-roots\round2
未声称 Miri、其它平台或真实设备验证。

Notify/waiter/task/waker 内部、原 stop watch、公开 native handle Arc/Weak、
opaque report/cause/wrapper、其它 marker、SDK/发现和 metadata 副本、
HTTP/WS/配置/凭据、框架/allocator 与完整 endpoint 恢复继续原范围。
128MiB 是已准入已知对象池，非完整 heap/RSS。L02 未关闭，23交付/6缺开发/
L02待全审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
