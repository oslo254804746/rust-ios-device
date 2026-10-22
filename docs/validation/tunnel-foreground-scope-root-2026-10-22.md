# L02 foreground NativeScope shared root · 2026-10-22

前台NativeScope原Arc<Inner>有独立实际共享分配，克隆和清理失败会保留
该根；Registry slots和ec76102的完整connect F没有支付它。本项将该根
改用既有shared_allocation::Shared<Inner>，同一原Registry byte_bank先
精确准入声明的完整Allocation<Inner>布局（计数、Inner、permit与padding），
再分配。未猜测std Arc私有header。new仍创建原Registry，from_registry
在Inner/连接semaphore构造前完成新的root费用准入；拒绝不关闭传入的
原Registry或创建native slot。Scope clone共享同一个实际根与原费用。

Inner自身是Unpin，没有对外scope地址/Weak/pin接口；复用既有Shared
最后根先释放原声明allocation、再销毁移动出的原Inner、最后返还费用
的机制。原Inner Drop仍close_and_stop_all，device→lease→permit Drop、
connect/prepare_connect/connect_admitted、collect/close及清理错误wrapper
正文逐字保持。实际Registry/native handle与joins不替换。代码未新增
unsafe/task/clock/bank/限额；shared_allocation生产实现完全不变，仅将
cfg(test)实际free观察助手开放给同模块域测试。该根与原Registry在
构造峰值都分别保有整笔费用，未用同一grant替两份实际backing付费。

新增3项测试：

- N−1与closed0在Scope backing前拒绝，零受控allocation；原Registry
  不sealed、occupied0、strong count恢复，独立scope失败不触发Inner Drop，
  其余原bank全部归还。
- 实际完整Root布局只分配一次且其Inner位于同一span；精确扣至0、同一
  Registry bank、三份Scope clones保持同一实际根/费用。前两份Drop不
  关闭Registry、不退款；closed0最后根实际物理free后的观察点费用仍
  在且Inner Drop尚未运行，随后原Inner关闭Registry，准确退同一root
  费用，最终原bank全额恢复。未把强计数减少当成物理释放。
- 实际本地userspace runtime已attach/complete，zero-budget close返回
  原RetainedScopeFailure，其中持有同一Scope根。释放原Scope后费用仍
  在；closed bank下释放实际错误所有者才退该root费用。独立原lease仍
  匹配原handle；使用原Registry继续原生joins，closed+empty、严格Ok(0)
  peer EOF和任务基线完成，最后lease/Registry费用全部归还。

真实cleanup错误测试里close已经请求stop；没有把错误最后Drop冒称一次
新观察到的首次stop。原错误字段/source方法不变有独立源码guard；这项
测试观察实际Scope根保留和原native joins，不声称替代其它typed IO回归。
所有实际runtime使用duplex，不poll Native SDK或设备发现/配对/接口路由。

R1 strict仅命中新测试err().expect()的err_expect检查，等价改expect_err，
无生产/断言语义变化或allow。独立只读审查核对R2三个source SHA，无需修改。
round2全部16门禁通过，全部Cargo/build主控串行：

- scope-root过滤：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1835 filtered out; finished in 0.01s
- shared-root过滤：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1831 filtered out; finished in 0.00s
- foreground-all过滤：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1828 filtered out; finished in 0.03s
- ownership过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1830 filtered out; finished in 0.01s
- native-retirement过滤：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1728 filtered out; finished in 43.41s
- lifecycle过滤：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1809 filtered out; finished in 6.67s
- intent过滤：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1827 filtered out; finished in 1.53s
- http-control过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1833 filtered out; finished in 0.04s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1834 filtered out; finished in 0.00s
- cli-full过滤：test result: ok. 1825 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.54s

另含CLI all-features/all-targets strict Clippy -D warnings、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。705冻结源码无漂移，3处源码差异仅CLI（含1新增，
shared实现仅cfgtest助手可见性）；前台6项frame测试源码保持。Core/lock
全量hash与ec76102/ec25c3e一致，沿用ec25已验24门禁/Coretunnel159/
reservation2/initial1/activation5/无mDNS2，未声称本轮重跑Core。
实际host及归档verified-ios.exe SHA-256：
927ba33fdf7c781e842ae09cf409a38cb7c952403dbf2e5c7661c8b8125bd15e
完整命令/log/result、source delta、current/提交后rawGit proof与实际二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-foreground-scope-root\round2

此项仅支付实际Scope共享根。其独立Arc<Semaphore>控制分配、外部返回
NativeConnection/SDK/credential/metadata/opaque errors、public Core Arc/
Weak/watch、HTTP/framework/task/transport/allocator/waker/waiter、
run_tunnel_start完整外F和原marker根/容量/same-bank来源/并发峰值证明、
所有模式endpoint恢复/native exposure事务仍在原范围。未以测试替身
豁免这些边界；128MiB仍是已知对象准入池，非整个heap/RSS。未做Miri/
其它平台/真机验收。主项23交付/6缺开发/L02待全审计、有效30/Q04全部
排除不变，仅本地提交并继续原七主项。
