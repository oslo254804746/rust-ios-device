# L02 standalone start concrete future · 2026-10-22

run_tunnel_start原opts/mode/诊断输出及Registry::new前缀保持；之后将原
StandaloneOwner、Ownership、具体Core连接future、device、lease、原同步
输出/flush、CtrlC等待和terminal drain放进同一个实际泛型run_admitted F。
原Registry byte bank在first poll前支付size_of::<F>()并分配精确capacity1
backing，复用PaidFuture原同指针Vec→Box实现，未新增unsafe或dyn future。
实际F连同其backing销毁之后才退费；准备失败和未轮询Drop均不进入SDK。

NativeConnector同步方法直接返回原connect_with_owner具体future，保留
CoreError直到原timeout后的??转换。NativeReporter原输出行和flush顺序
保持；其io错误在原primary中转换。CtrlC构造提前但尚未poll，锁定Tokio
实现的监听注册在first poll，仍发生在输出/flush成功后。生产SDK/CtrlC
只做构造与unpolled Drop验证，没有轮询真实设备连接或安装信号监听。

原30s timeout只覆盖connect，在Ownership成功后首次poll时创建。原
StandaloneOwner在Ownership之前创建，其早退?行为保留。primary中
device先声明、_lease后声明，因此正常/错误/取消路径仍先退役lease再
Drop device；未套用具有不同显式析构顺序的NativeConnection。原2s
cleanup clock仍在primary结束后创建，继而close→drop Ownership→drain→
原三分支match，主因/清理次因各保留原typed source和指针。prefix、
output、primary替换边界、cleanup和StandaloneOwner都有源码相等guard。

新增8项离线测试：

- 两种TunMode与script开关的实际native F布局：N−1/closed0准备拒绝且
  零backing；成功只分配整F一次，unpolled Drop全退且Registry仍open/
  empty。NativeConnector及原SDK future从未poll。
- 真实align64、!Unpin本地Connector和align128、!Unpin Signal作为F字段；
  Connector跨重复pending保持同一paid allocation span。Source只一次，
  report/signal尚未poll；closed0取消销毁实际F所有者并精确退款。
- duplex真实userspace native在Source pending及Signal pending两阶段
  caller Drop均退役原句柄；后续原terminal drain完成实际joins，closed/
  empty、严格peer.read==Ok(0)及任务基线全部核对，无真实设备。
- 成功路径同原handle handoff，report先于signal；Signal地址仍在原F
  span。暂停时钟推进31s仍pending且原native存活，证明connect timer
  不覆盖Signal等待。closed0放行后原lease→device析构、terminal joins/
  严格EOF和准确fee退款通过。
- prepare后推进40s尚无Source/Ownership slot，first poll后29s仍pending，
  31s返回原30秒超时原文；未改成新Elapsed分类或扩展时钟范围。
- paid F后首次poll Ownership closed0拒绝，Source/report/signal均0，
  原StandaloneOwner早退关闭空Registry，仅整F退款，没有新清理包装。
- Source、report、signal三阶段各注入原IO，closed0下保留CoreError或
  io::Error原类别、堆内Original及更深IO字段地址；原native joins和
  严格EOF/任务基线通过。Reporter为本地同步fixture。
- 另一三阶段矩阵用真实duplex EOF故障产生native错误。pre-drain
  snapshot明确仍有未join所有者；保存原work pointer。原terminal drain
  后CleanupFailure保留primary及单独cleanup，后者真实joins_complete
  且同work pointer，不把worker finished冒称join完成。

round1 strict Clippy通过，7项测试6通过/1失败：初版测试在drain前错误
断言joins_complete，已按Core实际owned-join契约修正；未改生产代码。
随后补齐signal>30s与Ownership早退测试。round2全部15门禁通过，
所有Cargo/build由主控串行执行：

- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1844 filtered out; finished in 0.05s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1836 filtered out; finished in 0.04s
- ownership：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1844 filtered out; finished in 0.01s
- native-retirement：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1742 filtered out; finished in 43.85s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1823 filtered out; finished in 6.68s
- intent：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1841 filtered out; finished in 1.53s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1847 filtered out; finished in 0.04s
- future-storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1848 filtered out; finished in 0.00s
- cli-full：test result: ok. 1839 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.97s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。708冻结源码hash无漂移，3处CLI源码差异含
2新增；Core/Cargo.lock相对1dad/ec25全部hash保持，沿用ec25已验Core24
门禁，未声称本轮重跑Core。独立只读设计与实现审查及主控原body guard
通过；审查者未运行Cargo/测试/编辑。实际host与归档二进制SHA-256：
f015f4dd269d16c72860471038184e87eb0cad9659f62357f7a7e1c954e2da4a
完整命令/log/result、冻结inventory、source delta、提交后rawGit proof与
verified-ios.exe：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-standalone-frame\round2

NativeReporter真实stdout字节/flush故障和missing handle/address/port分支
本轮未单独执行，保留原句的源码guard与既有回归不等于这些新动态测试。
整个CLI外层/HTTP框架/task cells、原设备/SDK/credential/metadata/opaque
errors、public Core Arc/Weak/watch、allocator/waker/waiter及transport堆
仍需原范围证明。完整marker census/same-bank keeper/producer/Weak/
控制峰值/arena容量与所有模式endpoint恢复/native exposure事务仍未闭合。
本项支付实际F allocation，不声称其内部Arc/Box/SDK heap自动被支付。
128MiB仍为已知对象准入池，非整个heap/RSS；无Miri/其它平台/真机验证。
有效30/Q04全排除、23交付/6缺开发/L02待全审计不变，继续原七主项目标。
