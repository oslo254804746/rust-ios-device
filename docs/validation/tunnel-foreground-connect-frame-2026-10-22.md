# L02 foreground NativeScope concrete connect F · 2026-10-22

NativeScope::connect供forward、HID、viewer、test/WDA等前台命令使用，原完整
async F内持有Ownership::standalone的原stamp、native reservation以及完整
SDK连接future。固定4个connection permits和Registry slots的费用没有支付
这份具体F。本项保留原selectors→原4连接permit前缀，随后由原Registry
byte_bank精确准入整个connect_admitted<C>实际Box，再首次poll。构造Owner/
stamp和调用SDK均发生在该paid F内部。新的metadata ResourceLimit可以在
尚无Source/slot的首次poll前拒绝；selector和原4连接容量错误仍先返回。

生产NativeConnector继续原ios_core::connect_with_owner(udid, options, owner)，
相同tun_mode/pair_record_path=None/skip_tunnel=false，没有新clock、task、
bank、限额、retry或原生连接策略。private Connector返回具体impl Future，
完整Core future及C本身都包含在同一F中，未擦除/额外装箱。测试连接器也
使用同一个TunnelReservationOwner/attach/complete/take_completed代码路径。

NativeConnection增加默认D=ConnectedDevice以复用同一个输出所有者测试；
生产connect返回签名、实际字段和Deref目标保持。device→lease→permit
销毁顺序、NativeScope Inner Drop、collect/close正文与ec25c3e逐字相同。
原Ownership到NativeConnection返回正文仅将原Core调用交给具体connector；
空格归一化guard核对其余原代码保持。新准入使用既有PaidFuture，没有新增
unsafe。外层小async入口只保留参数/paid owner，原stamp位于付费完整F。

新增6项测试：

- 测量实际NativeConnector完整F（只构造、零SDK poll），N−1及closed0拒绝
  时零受控allocation/slot、连接permit全退。超长selector与4许可全占时，
  原prefix错误仍先于byte准入错误，registry不关闭/不变。
- 实际原生具体F只有一份精确backing。未poll Drop不创建Ownership/新stamp/
  native slot、不调用SDK；原F与连接permit准确全退，原Registry仍开放。
- 对齐128、!Unpin连接器的原129字节payload与实际self地址位于同一完整
  F span。重复pending poll和closed0不移动该地址、不重建连接器；实际F
  Drop（非仅Pin引用Drop）才析构连接器并退F/Attempt/空incarnation费用。
- 真实本地userspace runtime已attach而Source pending时，closed0调用者
  Drop准确退F/Attempt费用，原handle先退休且Entry/incarnation直到原joins
  后才释放；严格Ok(0) peer EOF、任务基线、原bank全额恢复。
- 成功返回保留同一个原handle及匹配的原lease、原连接permit。F和Attempt
  释放独立于返回device；close完成实际joins后device仍持原incarnation，
  device Drop时其费用仍在，随后lease/permit释放，最终全额退款。全顺序
  由原样显式Drop源码证明，未将close先完成的测试冒称逐事件析构观察。
- Ready原始PermissionDenied IO在closed0下仍保留原typed root和深层IO
  实际地址，不被新增ResourceLimit覆盖；已attach原handle退休、原joins/
  严格EOF及任务基线均完成，原bank全退。

SDK真实连接路径只构造实际具体F、未poll；所有实际runtime/网络均为本地
或duplex fixture，不做设备发现/配对/接口路由操作。未把Source替身通过
声称为真实设备接受。只读独立审查和原源码guard另存外部证据目录。

round1 首轮全部15门禁通过，全部Cargo/build主控串行：

- foreground-frame过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1829 filtered out; finished in 0.02s
- foreground-all过滤：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1828 filtered out; finished in 0.02s
- ownership过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1827 filtered out; finished in 0.01s
- native-retirement过滤：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1725 filtered out; finished in 43.83s
- lifecycle过滤：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1806 filtered out; finished in 6.68s
- intent过滤：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1824 filtered out; finished in 1.62s
- http-control过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1830 filtered out; finished in 0.05s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1831 filtered out; finished in 0.00s
- cli-full过滤：test result: ok. 1822 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 246.29s

另含CLI all-features/all-targets strict Clippy -D warnings、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。704冻结源码无漂移，2处源码差异仅CLI（含1新增）。
Core/Cargo.lock全量hash与ec25c3e一致，沿用其已验24门禁/Coretunnel159/
reservation2/initial1/activation5/无mDNS2；未声称本轮重跑Core。
实际host及归档verified-ios.exe SHA-256：
89e98df9ff52a3b0aec60585108deb9706286c6b19e8213881b9ceeae60f6a02
完整命令/log/result、source delta、current/提交后rawGit proof与实际二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-foreground-connect-frame\round1

三份ec25c3e只读marker census/bank/peaks报告指出这个真实包含F缺口，现仅
该NativeScope路径已实现。完整marker arena容量、same-bank keeper/producer
证明、Weak/控制并发峰值仍未证明；run_tunnel_start另一条standalone外F、
NativeScope Arc<Inner>、返回device/SDK/credential/metadata/opaque errors、
public Core Arc/Weak/watch、HTTP/framework/task/transport/allocator/waker/
waiter以及各模式endpoint恢复/native exposure事务仍属原范围。128MiB
仍是已知对象准入池，非整个heap/RSS。未做Miri/其它平台/真机验收。
主项23交付/6缺开发/L02待全审计、有效30/Q04全部排除不变，仅本地提交并
继续原七个主项。本项并不完成L02或目标。
