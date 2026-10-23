# L02 authentic marker birth-bank TOTAL · 2026-10-23

原raw Arc<Semaphore>只提供available_permits，持有grant会降低该值，
退款会提高该值；该数不是总容量。本项新增私有inline BirthBank，
唯一new(total)同时创建原raw semaphore并保存不可变original total。
没有from_raw、raw+total重贴、total setter、数值epoch或额外wrapper
Arc/heap。普通allocator/Core仍借同一raw Arc，指针bank判断保持。
raw借用不证明任意外部mutation受封装；本项保存的是原出生容量。

这不是孤立primitive：State实际global factory与fixture rebind、
Lifecycle构造/所有global/device/scan births、desired Cell构造/reset/
replace/checked_replace，以及Registry构造/rebind和standalone两身份
出生全部使用同一真实authority。Identity/Clone/cfg Weak upgrade携带
原authority，旧captured identity不随新State/Registry配置重标。
现有15个new表达式的原控制位置和顺序保持，包括standalone原一处
两身份。原Identity实际std Arc分配与指针相等/Weak协议保持，底层
physical arena仍未实现，不猜std Arc header费。

Ticket实际origin-grant入口与native proof/catalogue mirror收费现在
调用captured identity.origin_charge。其真实BirthBank先按原u32转换
顺序处理overflow，再用immutable TOTAL拒绝不可能的完整root费用，
最后执行同raw semaphore的原try_acquire。raw兼容入口仍走同一实现
且不加TOTAL限制。原snapshot_limit/global known Resource消息、typed
class、去重、covered实际P permit/count、grant staging及partial rollback
不变。普通body/backing按真实新类型布局计费；Identity每个inline
authority带total字段，原size_of/body_bytes自动计入watch/gate/registry/
Ticket/owner等真实布局，不增生产容量，不加逻辑占位费。

旧测试在原容量处创建BirthBank并保留原raw别名；producer helpers
接收authority，allocator/grant/Source等仍接raw。原断言、IO指针、
native joins/strict EOF、epoch/clock/counter/backoff/max及容量边界保持。
唯一原N−1→N测试（failure destination）改为new完整N并持有真实1字节
permit，原拒绝后drop该permit再成功，避免add_permits改变出生容量。
其它Source gate/信号/独立count池及未作为marker producer的raw银行
接口没有泛改；无cleanup/clear/stop late quota或控制等待。

新增3项离线检查：

- 原TOTAL17在held16/实际origin fee1、held退款、Clone/真实Weak
  upgrade、原authority Drop及closed后保持原容量语义。17许可最终
  全退，closed仍拒绝新origin fee，实际身份/原bank指针保持。
- bytes18及usize::MAX分别与原raw入口比较错误结果/ResourceLimit，
  保持total拒绝与原u32 overflow优先；exact17 grant持原semaphore。
- 实际Lifecycle与Registry共享一个authentic出生bank。Registry唯一
  empty rebind后用新bank，而旧captured Ticket实际publication root
  仍向原bank支付整费。old bank零余量/closed下clear保持原失效行为；
  新Registry结束新bank退款，旧Ticket根最后结束后原bank全退。

主控逐体源码guard核对State/Lifecycle/Cell/Registry所有原Source、
控制通知、native handle/joins/Drop、ordinal/capacity与时钟体，只有
明确构造/类型与origin收费入口迁移。24个原测试文件/inline测试的
factory/borrowed raw alias投影后与原断言体一致；保留N−1→N的明确
held1差异，未通过删断言、放大fixture或增生产pool换取通过。
只读Agent提供过集成设计和原22SHA，实施由主控完成；没有独立最终
实现批准，不把设计报告当作当前实现测试。初次all-target check有19
处旧辅助接口迁移错误；第二次通过但有两个未使用测试import，随后
在最终freeze前清理。最终同720冻结源码的40门禁首轮全部通过：

- birth-bank：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1880 filtered out; finished in 0.00s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1836 filtered out; finished in 0.18s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1854 filtered out; finished in 6.76s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1873 filtered out; finished in 0.00s
- connection-ticket：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1877 filtered out; finished in 0.06s
- upgrade-owner：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1878 filtered out; finished in 0.00s
- upgrade-epoch：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1877 filtered out; finished in 0.30s
- publication-ticket：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1878 filtered out; finished in 0.01s
- native-retirement：test result: ok. 110 passed; 0 failed; 1 ignored; 0 measured; 1772 filtered out; finished in 42.80s
- publication-all：test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.01s
- lifecycle-all：test result: ok. 335 passed; 0 failed; 0 ignored; 0 measured; 1548 filtered out; finished in 85.20s
- receipt-origin：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1880 filtered out; finished in 0.00s
- episode-ticket：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1880 filtered out; finished in 0.00s
- origin-census：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1880 filtered out; finished in 0.00s
- origin-native：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1879 filtered out; finished in 0.01s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1866 filtered out; finished in 0.05s
- reconnect：test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1830 filtered out; finished in 6.84s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1870 filtered out; finished in 0.11s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1874 filtered out; finished in 0.01s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1867 filtered out; finished in 0.04s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1875 filtered out; finished in 0.04s
- cli-full：test result: ok. 1870 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 243.88s

其余identity/shared weak/manual/error payload/failure slot/manager owner/
intent route/HTTP/scan/ensure/future/publication-retain定向门禁也通过，
另有strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。全部Cargo/build/fmt/test由主控串行执行，
测试--locked/--test-threads=1。720冻结hash无漂移，33处仅CLI变化
（8生产/25测试，含新BirthBank及新测试2文件；ownership.rs仅cfg测试
迁移），Core/Cargo.lock保持247/ec25；旧Core24证据沿用、未重跑。
实际host与归档verified-ios.exe SHA-256 0380770e81821c07b9f65b9d244ef927ea272dab2a13259c3b84f663c8b5bbaf
完整gate logs/results、source inventory/guard/delta/current/raw HEAD
提交后proof及二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-birth-bank-authority\round1

本项只闭合真实出生authority及真实origin收费消费TOTAL这一前提，
未给arena选择任何容量。仍需所有actual holder/Weak/同步通知峰值
证明及完整物理marker backing最后free收费；failed Node独立global、
RefreshProof实际根、writer/snapshot/public/opaque/SDK/legacy/framework/
task/waker/waiter/allocator/native/transport与全部mode endpoint恢复/
native exposure仍属原scope。128MiB为已知对象准入池，非heap/RSS。
无Miri/其它平台/真实设备结论；23交付/6缺开发/L02待全审计、有效30/
Q04全部排除保持，七主项继续。
