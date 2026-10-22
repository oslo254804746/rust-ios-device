# L02 complete prepared server future · 2026-10-22

原 server async frame 包含 inline monitor、generic signal、HTTP server和
cleanup phases，实际存储未独立准入。同步私有 server::prepare 现在从同一
原 global bank 精确准入完整具体 serve_admitted<S> F 的原 Box/pin，再
返回该 future。S 完整保留在编译器实际布局内，不借用/erase signal来缩小
声明；不存在 legacy unmetered serve 分支。原方法从 let _owner 到最后
result 的正文与04e1518逐字比较一致，原独立cleanup clock、signal/server/
native/history优先级、monitor.set(None)、实际joins均保持。

Owner仍在实际future第一次poll时创建。prepare拒绝或已准备但未poll的
future结束不新启Owner，也不新通知/清空保留的manager；其输入资源照常
析构。已经poll的caller Drop继续原Owner::drop的close+clear零配额逻辑，
没有晚申请grant。F值与实际allocation释放后才退其独立费用。未新增
unsafe、task、bank、固定capacity或时钟；公共Core/API/binding未改。

这不是在旧终态报告前新增一次可能失败的准入。原 repeated_terminal_reports
回归先获得原paired IO/history，再耗尽额度，必须两次报告同一原对象。
因此调用方在Source之前准备两份真实完整future，之后在bank0时运行这些
原实例。每份F实际结束后新增的真实退款有exact frame_bytes断言，再把
该额度合并进同一原held permit，保留下一次运行及旧断言的零余额。原
history/root/deep IO指针、两次TerminalCleanupFailure、native closed/empty
与实际recovery/joins检查全部保留，未用ResourceLimit替代既有原因。

全部17旧调用点迁移：生产cold入口在listening日志前完成prepare；原native
server与list-error夹具在publish前准备，保留原signal/metadata锁时序；其它
夹具可独立完成此前成功setup；monitor的Source在首次poll原执行体后才
开始。未轮询future不会启动monitor或signal。
固定Generic S的sizeof helper通过未调用的原constructor类型推断，不构造
虚假工作或估算私有头。主控独立实现/审查，未使用子agent Cargo或实际设备。

新增5测试：

- exact N−1/有效闭池0拒绝，零受控backing/Source/signal poll，输入signal
  只析构一次，保留state仍open，listener实际释放。
- exact完整F在首次分配前扣零；64字节对齐 !Unpin signal 原数据保留，
  closed0未poll释放时signal先于费用结束，原Owner不启动。
- 原signal在实际F地址范围内poll两次并固定原地址到Drop，closed0没有新
  受控allocation；actual started Owner关闭state，实际listener结束。
- 实际monitor中pending FakeSource caller Drop，Source退出一次、scan锁
  释放；F与nested费用退款，仅保留原canonical gate费用，最后全部退款。
- 原signal Ready IO同一poll耗尽并关闭bank，cleanup后PermissionDenied与
  原深层SignalCause字节/地址不变，F实际结束返回exact费用；Source仍0。

round1 首轮全部16命令通过，全部Cargo/build由主控串行：

- server-frame过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1812 filtered out; finished in 8.16s
- server过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1809 filtered out; finished in 10.34s
- scan-outcome过滤：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1810 filtered out; finished in 0.15s
- native-server过滤：test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1815 filtered out; finished in 0.05s
- shutdown-admission过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1809 filtered out; finished in 2.06s
- scan-frame过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1812 filtered out; finished in 0.14s
- ensure-frame过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1811 filtered out; finished in 0.08s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1813 filtered out; finished in 0.00s
- error-payload过滤：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1808 filtered out; finished in 0.02s
- cli-full过滤：test result: ok. 1804 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 245.58s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。698冻结源码无漂移，8处仅CLI差异；Core/Cargo.lock
全部hash与04e1518状态相同，Core沿用846c533已验29门禁/tunnel159/owner42/
无mDNS37/watch10/无mDNS10，未声称本轮重跑。原server正文一致性与17旧调用点
迁移有独立guard。实际host及verified-ios.exe SHA-256：
70a0038bb97059ed89561b850bda030ee486442e98c827b8dcd633fa35c8a10d
命令日志、source delta/current/提交后rawGit proof与实际二进制归档：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-server-frame-owner\round1

该费用覆盖完整具体F；Axum/Hyper/HTTP delegated task cells、signal/runtime/
authority/transport深层heap、opaque error/report、marker自身根、公开Core
Arc/Weak/watch、SDK/metadata/config/credential/allocator/Waker/waiter及完整
各模式endpoint恢复/native exposure事务仍属原范围，未豁免。未声称
Miri/其它平台/真机或整个heap/RSS。128MiB仍是已准入已知对象池，固定限额
保持。主项23交付/6缺开发/L02待全审计、有效30/Q04全部排除不变。仅本地提交。
