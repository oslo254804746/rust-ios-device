# L02 failure-count semaphore root · 2026-10-23

原manager failure_slots是独立Arc<Semaphore>，failure Slot/receipt/错误
可通过owned count permit保留该分配到State结束之后。原Shared<Slot>
只支付Slot自己的完整分配，未支付该Semaphore独立根。本项用Counts
直接持有已知完整布局的Shared<Semaphore>，由原manager global bank G
支付其控制/值/fee/padding；不猜std Arc私有header，不用State根代替。

Counts::new先准入整笔费用并准备实际空Destination，再构造原Semaphore
并无失败place。State仍在原failure_slots字段位置调用new，容量仍为
MAX_FAILURE_RECORDS；此前字段和后续初始化顺序保持。早期构造失败
只销毁已初始化字段，此时native为空、Source未poll。现有manager
构造/准确最小预算/弱引用测试均保留，结构费用增加实际Counts根。

私有非Clone Permit只持Counts。取得仍由原Tokio try_acquire()裁决，
成功forget后立即无失败地构造owner，中间没有await/分配/回调。Drop
先add_permits(1)再释放Counts字段，保持最后一个实际许可归还同一
Semaphore。Slot仍是原Shared<Slot>，count拒绝优先于body bytes/分配；
body失败仍退回该原count。原先Slot的OwnedSemaphorePermit Deref仅
被测试用于num_permits和identity，现用测试专用固定1和Counts ptr_eq
核对；没有新增通用permit或异步等待接口。

Counts不持State/Weak State/Registry/native，因此保留原错误/Slot不会
推迟manager字段或native原清理时机。Slot clone仍只共享同一Slot根、
一个原count。Slot实际释放后其Permit归还count，独立Counts根由它的
最后owner释放，整笔根费直到实际物理free及值析构才退款。原Shared
最后释放实现不变；free时费用仍在由实际after_free观察点直接核对。

cfg(test) rebind只允许Counts唯一且自身Semaphore未closed；每个活着
的私有Permit都持一个Counts，strong1排除其存在。新精确grant先取得，
再用原Shared::replace_storage替换同一allocation的fee，最后退旧费。
它不新建/重置Semaphore、计数或指针；拒绝保留原根/费用。整个旧
set_global_metadata_limit本身仍是before-sharing的unwrap配置helper，
本项没有声称把它改成整体原子事务。

新增3项并更新原4项failure Slot测试：

- Counts N−1和closed byte bank拒绝，零backing；准确根只分配一次，
  原Semaphore值地址位于该真实allocation span，grant属于原bank。
- 在根backing分配回调中关闭已准入byte bank，仍完成Semaphore构造和
  place；三个count取得/耗尽/退款再取、Closed拒绝和最后owned许可
  生命周期保持。原同一根/值指针，最后物理free时整费仍在，之后全退。
- rebind在N−1、新bank closed、外部alias和活着的Permit时拒绝；成功
  无新allocation，原pointer及容量保持，旧bank准确退款，新bank接续
  整费。新byte bank关闭后原count许可仍可取得/归还，最后准确全退。
- 原Slot body N−1/closed、count优先、并行最后clone和同一原Slot
  identity仍通过。其独立count pools现在用显式有限fixture bank支付
  Counts根，保持原Slot-only byte quota，且根构造在精确hook之前。
- 实际manager的原Source IO/receipt/Slot在closed0重用，clear与State
  Drop后仍同原IO指针、原Slot指针；费用精确区分Slot根和Counts根。
  最后Slot归还count后，外部Counts观察者仍持其根费，直到它的实际
  free观察点后才全额退款。未冒称此测试attach了native。

两处manager计数容量fixture仍分别为1/2并用原G支付新根；其它原独立
count/byte池用明确的独立有限root bank。九处精确结构公式只增加真实
Counts::body_bytes，原Slot布局用实际Slot::body_bytes测量。没有扩大
生产容量或删除旧Source、retry计数、时间、错误、清理断言。

round1首轮全部20门禁通过，所有Cargo/build由主控串行执行：

- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1849 filtered out; finished in 0.01s
- manager-owner：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1852 filtered out; finished in 0.01s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1839 filtered out; finished in 0.07s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1843 filtered out; finished in 0.11s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1847 filtered out; finished in 0.01s
- reconnect：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1809 filtered out; finished in 6.82s
- native-retirement：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1746 filtered out; finished in 43.81s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1827 filtered out; finished in 6.71s
- intent：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1845 filtered out; finished in 2.46s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1851 filtered out; finished in 0.04s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1840 filtered out; finished in 0.04s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1848 filtered out; finished in 0.04s
- future-storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1852 filtered out; finished in 0.00s
- cli-full：test result: ok. 1843 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 246.45s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0），host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。708冻结源码hash无漂移，13处CLI源码差异，
2生产+11测试；Core/Cargo.lock全量hash与bb4/1501/ec25保持，沿用已验
ec25 Core24门禁而未声称当前重跑Core。

独立只读设计报告已读并核对rawGit三baseline SHA，未见固有阻断；实现
与10个机械fixture迁移由主控源码比对和本轮实际门禁验证。子Agent后续
额度中断，没有独立实现批准/测试，不把设计审查冒称实现验收。原parent
除字段/同位置构造/cfg rebind三处外保持，原reserve优先顺序/重用/
notify/stop/clear/native drain及Shared实现都有源码相等guard。
实际host及归档verified-ios.exe SHA-256：
5031bccf7e95a98de7efbbfe3f2493d359849bc31e2e015b11ab7a14196792df
命令/log/result、冻结inventory/source delta、提交后rawGit proof和二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-failure-count-root\round1

本项只闭合该独立Semaphore根的已知完整分配；不自动支付其byte bank
std Arc、原其它SDK/credential/metadata/opaque error/public Core Arc/
Weak/watch、HTTP/framework/task/allocator/waker/waiter/transport堆。
marker完整census/跨bank keeper/producer/Weak/control peaks/不可变容量
authority/arena backing证明与所有mode endpoint恢复/native exposure
事务仍属原scope。无新unsafe/SDK路径/真实设备/Miri/其它平台验证。
128MiB仍为已知对象准入池，非整个heap/RSS。有效30/Q04全部排除、
23交付/6缺开发/L02待全审计不变，继续原七主项目标。
