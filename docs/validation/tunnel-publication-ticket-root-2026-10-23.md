# L02 publication actual Ticket root · 2026-10-23

ManagedTunnel原inline Ticket依赖原Node存储收费；Change/Retired值可以
独立于Node继续存活。本项把该Ticket移到一个独立、实际分配的根，
原发布行、替换回滚及返回的Retired值均持同一个已准入根。外层Node
缩小后仍按实际Node布局计费，Ticket实际根单独支付完整G/P费用。

publication::TicketOwner复用原shared_weak_allocation::Strong<Ticket,
(OwnedSemaphorePermit, TicketOriginGrants)>。body_bytes从该完整实际
Allocation类型读取，包括控制计数、G/P permits、三个origin位置、
Ticket及padding。先准入G与原P整费，再按captured字段的真实producer
bank支付去重origin整费，随后才clone原Ticket和分配实际backing。
P覆盖通过实际持有且数目恰等于完整body的OwnedSemaphorePermit证明，
不接受仅标注bank的跳过依据；G/P及重复origin各只收费一次。
原origin_grants的None路径语义保持。拒绝没有Ticket backing，所有
已暂存grant自动回退。Clone共享同一根，最后实际free后才退全部费用。
wrapper无Weak/raw/mutable/Pin出口，没有新unsafe或逻辑占位费。

原connect_and_store顺序只在Ownership和metadata PreparedPair之后、
observed_route/Source之前加入该根准备。原Source、服务验证、native
publish/final fence、metadata attach、rollback及ManagedTunnel Drop体
保持，成功时把同一准备根move进row。原Ownership可能已占ordinal，
本项只声称根拒绝时Source/usbmux未执行、原native occupied为零，
不声称此前ordinal从未变化。clear/stop/rollback/late cleanup不新取quota。

新增5项离线检查：

- G/P/真实Lifecycle origin/desired origin四bank逐个N−1和足额closed
  拒绝，零backing，其他grant余额准确回退；错误覆盖grant数目也拒绝。
- 原P同时是真实identity producer时去重，G/P/C零余量下实际只分配
  一次完整根，Ticket位于实span，canonical key及captured身份保持。
  原Ticket/Lifecycle结束、各bank关闭后，最后根free回调仍观察到整费，
  回调之后准确全退。
- 原Table publish/rollback在closed零余量下不分配，恢复同一Ticket
  地址及key。Retired值在Table/Lifecycle结束后独立保留旧根，最后
  Drop才释放真实marker与各bank费用。该fixture明确提供2MiB以容纳
  原4096-slot表backing；没有改变生产table上限或生产pool容量。
- 实际manager的P精确差一字节在Source前拒绝，typed Admission与
  ResourceLimit保留，Source/usbmux/paired均0，无row/native占用/retry
  receipt，最终全部退款。
- 实际Fake manager成功发布后clear使原device析构一次；独立根仍
  保持旧captured身份/key且过期。State结束后P仅余实际Ticket根费用，
  最后根结束后G/P全额退款。新测试不代替原native joins/EOF回归。

旧精确配额只加入真实Ticket body：published-capacity、observed-host
差一字节前缀及Lease分配观察点、native zero-quota Source peak fixture
和确实随row结束的退款差值同步。原Life/Lease/early destination拒绝
仍在Ticket前，不增加Ticket项；原charged累计式不重复计费，native
notify+drain但row尚活的零退款断言保持。原joins、strict Ok(0) EOF、
IO/error深层指针、epoch、counter、绝对clock、backoff/max/whole total
均保留。控制源码guard与独立Agent只读review核对9个当前SHA，无剩余
可操作源码发现；Agent不运行Cargo/不编辑实现，执行结果由主控提供。

开发验证有真实fixture失败：focused1新5中1处root-only 128KiB不能
容纳原固定4096-slot backing；修正该测试为明确2MiB。focused2旧host
差一字节前缀漏新根，focused3原native102成功/7失败/1原ignored，均
对应新增实际fee的旧公式遗漏。每轮终态后才编辑，修正后重冻全部源码。
这些失败保留日志，不记为PASS；最终同一冻结源码的36门禁全部成功。
所有Cargo/build/fmt/test均主控串行，测试--locked与--test-threads=1：

- publication-ticket：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1869 filtered out; finished in 0.01s
- publication-all：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1864 filtered out; finished in 0.01s
- lifecycle-all：test result: ok. 332 passed; 0 failed; 0 ignored; 0 measured; 1542 filtered out; finished in 85.76s
- native-retirement：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1764 filtered out; finished in 43.61s
- episode-ticket：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.00s
- receipt-origin：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.00s
- shared-weak：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1870 filtered out; finished in 0.00s
- origin-census：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.00s
- origin-native：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1870 filtered out; finished in 0.01s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1827 filtered out; finished in 0.17s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1845 filtered out; finished in 6.72s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1864 filtered out; finished in 0.00s
- manual-success：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1861 filtered out; finished in 0.74s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1857 filtered out; finished in 0.05s
- reconnect：test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1821 filtered out; finished in 6.82s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1865 filtered out; finished in 0.01s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1861 filtered out; finished in 0.11s
- error-payload：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1865 filtered out; finished in 0.02s
- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1867 filtered out; finished in 0.00s
- manager-owner：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1870 filtered out; finished in 0.00s
- intent-route：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1863 filtered out; finished in 1.62s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1869 filtered out; finished in 0.04s
- scan-frame：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1869 filtered out; finished in 0.14s
- ensure-frame：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1868 filtered out; finished in 0.08s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1858 filtered out; finished in 0.03s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1866 filtered out; finished in 0.05s
- future-storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1870 filtered out; finished in 0.00s
- publication-retain：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.06s
- cli-full：test result: ok. 1861 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 245.31s

另有strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。715冻结源码全部hash保持；9处CLI差异
（3生产/6测试，2新增测试文件）。Core/Cargo.lock与aee/ec25一致，
旧Core24门禁证据沿用，本轮未重跑Core。host执行binary与归档均为：
SHA-256 27d28c3f6269f837ca12ad8d09483115dafd7ec0b479ccbdef4ed7e68e63ae84
完整命令/log/result、frozen inventory/source-delta/current/提交后rawGit
proof、独立review及verified-ios.exe：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-publication-ticket-root\round1

本项仅覆盖ManagedTunnel/Change/Retired publication Ticket实际根。
LocalTarget::for_connection的独立raw Ticket、其它snapshot/frame/
opaque/public Core/SDK/legacy/HTTP/framework task/waker/waiter/allocator/
native/transport仍属原scope，未自动获得根收费。底层marker std Arc
physical arena、all-retainer/Weak/control峰值与不可变TOTAL authority、
全部mode endpoint恢复/native exposure仍待完成。128MiB为已知对象
准入池，非完整heap/RSS；没有Miri/其它平台/真实设备结论。
23交付/6缺开发/L02待全审计、有效30与Q04全部排除保持。原七主项继续。
