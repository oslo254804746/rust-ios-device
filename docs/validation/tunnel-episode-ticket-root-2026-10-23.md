# L02 shared episode/receipt actual Ticket allocation · 2026-10-23

Episode原先inline持Ticket，外层Removed Box可通过into_inner先free而
Episode/Ticket继续存活。只把origin费用绑在外层Box会过早退款，直接
在Episode中放一个逻辑占位费也不能说明实际分配。本项让episode与
receipt共享一个实际预付的Ticket allocation，费用随真正所有者保留。
不新增逻辑占位费，不改变外层Box的原物理释放协议。

TicketOwner直接复用既有shared_weak_allocation::Strong<Ticket,
TicketOriginGrants>。body_bytes从实际Allocation<T,E>完整类型读取，
含控制计数、B grant、真实origin grant数组、原Ticket字段及padding。
构造先准入B与每个captured foreign origin整费，再创建原Ticket clone
并实际分配；拒绝无backing且全部暂存费用回退。没有新unsafe、任务、
时钟、重试或SDK路径。该私有wrapper仅暴露Clone/借用Ticket，无Weak、
raw、mutable或Pin出口；本项没有改变原通用分配器或Ticket控制逻辑。

FailureReceipt在原capsule各B grants及receipt origin grants之后，
selector复制之前，准备实际Ticket根。其值持TicketOwner而非inline
Ticket；其自身完整布局费用按真实类型同步变化，既有receipt费用仍
保留到最后Weak实际free。新episode只克隆这份已准备的Ticket根，
同episode后续retry保留首个原captured Ticket，后续receipt使用各自
当前captured Ticket根。实际根没有receipt、IO、Ledger或row反向引用，
不会为了保持Ticket而延长旧receipt/Source错误的寿命。

Removed::into_inner仍按原协议free外层Box再返回值，Episode内指针及
独立实际Ticket allocation保持；原B Box body费准确退款，Ticket根
在其最后实际所有者结束后才free/退B及各origin费。原canonical key、
route/string buffer、native identity、epoch fence及原retry first-attempt/
deadline/backoff/max/counter/SourceFailure历史保持；未把新根当作新的
generation或重启episode。

新增3项离线验证及原精确配额迁移：

- B N−1、足额closed及origin N−1拒绝时零Ticket backing、原A/B余额
  保持。准确A/B零可用下只分配一次完整layout，Ticket值位于实际span，
  captured身份和canonical key原指针保持；Clone同根无再次分配。
  两bank关闭、原Ticket/Lifecycle结束后，最后根物理free回调仍持
  整笔A/B费，之后准确全退。子watch free事件分别观察，不假装只有
  一个全局allocation。
- 原Removed Box释放后其返回Episode仍持相同Ticket实际地址及key
  buffer，into_inner零新allocation，只退原outer body。A及独立Ticket
  根B费用仍持有，最终值结束后全退；原selectors指针和旧测试保留。
- 实际Ledger首次episode与receipt同Ticket根，第二次同episode retry
  的receipt另有当前根而episode保持首根。首attempt/deadline不变，
  attempts只按原规则递增。原Work及深层IO字段地址保持；首receipt
  Weak在合法history仍持IO时已不能upgrade，证明episode只持Ticket。
  history最后释放触发原IO析构一次，此时episode/Ticket仍活。移出的
  Removed.into_inner继续保留原根，最后准确全额退款。

原selector预付测试新增实际Ticket allocation观察并保留原text/host/
capsule指针与准确原layout断言；总预算只增加真实TicketOwner::body_bytes，
不增生产容量。原receipt optional-origin用例同时核对Ticket与receipt
两份实际根：last strong结束退Ticket根，其后仍有receipt body费到
最后Weak free。旧Removed、receipt、retry、native joins/EOF及IO回归
仍执行，没有用新测试替换旧max/时钟/失败断言。

首次test-target check通过（临时未使用的测试ptr_eq随后由真实身份
测试调用）；focused1严格Clippy及5组测试全部通过。最终round1已完成
33门禁；其完整CLI首次运行中断、句柄丢失且没有结果文件。经进程
核查确认无Cargo/runner/test binary运行，713冻结源码hash均匹配，
保留cli-full.interrupted.log后只恢复尚未完成的6道门禁。该无终态
部分运行不计PASS，也不计测试失败；已有27道结果未重跑。
所有Cargo/build由主控串行执行，最终结果如下：

- episode-ticket：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1866 filtered out; finished in 0.00s
- receipt-origin：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1866 filtered out; finished in 0.00s
- reconnect：test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1816 filtered out; finished in 6.83s
- shared-weak：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1865 filtered out; finished in 0.00s
- origin-native：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1865 filtered out; finished in 0.01s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1822 filtered out; finished in 0.19s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1840 filtered out; finished in 6.72s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out; finished in 0.00s
- manual-success：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1856 filtered out; finished in 0.77s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1852 filtered out; finished in 0.05s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1860 filtered out; finished in 0.01s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1856 filtered out; finished in 0.11s
- error-payload：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1860 filtered out; finished in 0.02s
- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1862 filtered out; finished in 0.01s
- manager-owner：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1865 filtered out; finished in 0.00s
- native-retirement：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1759 filtered out; finished in 43.73s
- intent-route：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1858 filtered out; finished in 1.79s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1864 filtered out; finished in 0.05s
- scan-frame：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1864 filtered out; finished in 0.14s
- ensure-frame：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1863 filtered out; finished in 0.08s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1853 filtered out; finished in 0.04s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1861 filtered out; finished in 0.05s
- future-storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1865 filtered out; finished in 0.00s
- publication-retain：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1866 filtered out; finished in 0.06s
- cli-full：test result: ok. 1856 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 248.78s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。713冻结源码无漂移，8处CLI差异（3生产/
5测试，1新增测试文件），Core/Cargo.lock与c9/ec25全部hash保持；旧Core24
证据沿用，未称本轮重跑Core。主控源码guard核对原lifecycle/receipt
IO-history-completion、retry admission/counter/max/backoff/deadline及原
Box into_inner/unsafe分配协议全部保持，仅明确root type/clone及实际
配额和分配顺序迁移。没有独立Agent实现批准。实际host及归档二进制：
SHA-256 967f7973e25aaee8a167ad285acf0aeb8a8030d30134e68fc0598ffe8252cb44
命令/log/result、冻结inventory、source guard/delta/current/提交后rawGit
proof、恢复核查和verified-ios.exe：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-episode-ticket-root\round1

本项覆盖episode及receipt共享Ticket的实际根寿命，不声明其它任意
Ticket clone/returned snapshot/opaque keeper自动支付origin，也未实现
底层marker Arc physical arena。全retainer/Weak/control峰值/不可变TOTAL
authority、publication/frame/public Core/SDK/legacy/HTTP/framework/task/
allocator/waker/waiter/transport和全部mode endpoint恢复/native exposure
仍属原scope。无Miri/其它平台/真实设备验证；128MiB仍是已知对象
准入池，非整个heap/RSS。23交付/6缺开发/L02待全审计、有效30/Q04
全部排除保持，原七主项目标继续。
