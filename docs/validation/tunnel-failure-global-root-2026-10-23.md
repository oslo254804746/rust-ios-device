# L02 failed row actual global-identity root · 2026-10-23

FailedTunnelAttempt.global原来持raw ControlIdentity，可在receipt与episode
根结束后独立存活；那些对象的origin费用不能用于证明此字段。本项用
独立实际Identity根替代该字段，费用随真实root保持，与外层Node Box
和receipt/IO寿命分离。

marker_identity::Owner复用原Strong<Identity,Option<OwnedSemaphorePermit>>，
body_bytes从完整实际Allocation类型读取，G全费、真实captured foreign
origin A全费先准入，随后才clone原Identity及分配实际backing；同producer
只需一份G费。真实birth authority/TOTAL、原bank/pointer identity和
底层std Arc/Weak协议保持。Clone共享实际根，Debug直接转发原Identity，
最后root实际free后退G/A。没有新unsafe、数值epoch、占位费或API去取
mutable/raw/Weak。根只持Identity，无Ticket/gate/watch/receipt/IO/State/
table/native反向引用。底层marker std Arc物理收费仍是独立arena工作。

Node新prepared_global初始None，row.global在原commit前仍None。原四处
Node::input规划可在Ticket之前，均保持批量准备和ticket获取原次序。
实际refresh_connect_with_input在原existing/skip检查之后、failure_slot
预留之前，从Ticket原shutdown_generation借用准备root；没有先造一个
raw global clone再收费。拒绝时failure_slot仍None、Source未构造/poll，
旧row不受新失败写入影响。正常失败提交在原fence下仅将同一prepared
root移到row.global；没有新quota/clone/allocation。原计数、时间、
FailureVersion/slot、native/IO和rollback体保持。

record_prepared_failure及生产bookkeeping writer没有晚准备/收费兜底。
真实Source direct测试路径已在其候选前置阶段绑定，测试seed helper
和两组纯writer fixture在原setup阶段绑定。成功或NoChange只析构
未使用的prepared root；into_success_key仍转移原key/input费用。
外层Node真实布局按新字段自动计费，独立root另计实际费用。

新增5项离线检查覆盖：G/A分别N−1/足额closed零backing及partial费用
回滚、拒绝不clone Identity；实际span/原身份/Clone同根及最后free时
费用仍持有；同producer只付一份实际root费用；真实Node提前准备后
closed零余量commit仅move同地址root，Node/Ticket/Lifecycle结束后
仅余独立root的G/A费用，最后全退；实际manager候选精确差一字节拒绝，
Source/usbmux/paired/slot/row/native均未进入，部分prefix全退。旧typed
IO/deep指针、closed零余量failure写入、retry clocks/count/max、native
owned joins及严格Ok(0) EOF回归继续执行。

主控源码guard逐体核对原Source/clock/counter/error/native/rollback体，
确认原Node batch准备顺序、晚writer没有quota且commit仅move。原分配器
unsafe协议、birth authority、publication/upgrade等完整hash保持。首次
strict compile缺少新Owner的Debug约束，终态后补原Identity直接转发；
随后重冻全部723源码。没有独立Agent最终实现批准，主控复核与执行。
最终41道串行门禁全部通过：

- failure-global：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1883 filtered out; finished in 0.01s
- failure-table：test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1874 filtered out; finished in 0.11s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1879 filtered out; finished in 0.01s
- lifecycle-all：test result: ok. 336 passed; 0 failed; 0 ignored; 0 measured; 1552 filtered out; finished in 85.50s
- native-retirement：test result: ok. 110 passed; 0 failed; 1 ignored; 0 measured; 1777 filtered out; finished in 43.12s
- birth-bank：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1885 filtered out; finished in 0.00s
- connection-ticket：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1882 filtered out; finished in 0.06s
- publication-ticket：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1883 filtered out; finished in 0.01s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1841 filtered out; finished in 0.18s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out; finished in 6.73s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1878 filtered out; finished in 0.00s
- reconnect：test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1835 filtered out; finished in 6.83s
- receipt-origin：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1885 filtered out; finished in 0.00s
- episode-ticket：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1885 filtered out; finished in 0.00s
- error-payload：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1879 filtered out; finished in 0.02s
- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1881 filtered out; finished in 0.01s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1872 filtered out; finished in 0.03s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1880 filtered out; finished in 0.04s
- cli-full：test result: ok. 1875 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.41s

其余upgrade owner/epoch、identity/shared weak/origin/manual/global/manager/
intent route/HTTP/scan/ensure/future/publication-retain定向门禁也通过，
另有strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0），host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。全部Cargo/build/fmt/test均主控串行；测试
--locked/--test-threads=1。723冻结源码hash无漂移，11处仅CLI差异
（5生产/6测试、3新增测试文件），Core/Cargo.lock与bd4/ec25保持；旧
Core24证据沿用、本轮未重跑。实际host与归档verified-ios.exe：
SHA-256 7a2105e060f6ce10332b5ac4c463926953498da05953923008b116da0e5f095d
完整gate commands/logs/results、frozen/guard/delta/current/提交后raw HEAD
proof与二进制归档：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-failure-global-root\round2

本项覆盖独立failed-row global identity根；其它writer/snapshot/inline
RefreshProof持有者、全部physical marker arena/Weak/同步通知峰值及
opaque/public/Core/SDK/legacy/framework/task/waker/waiter/allocator/native/
transport、全部mode endpoint恢复/native exposure仍属原scope。128MiB
仍是已知对象准入池、非heap/RSS；无Miri/其它平台/真实设备结论。
23交付/6缺开发/L02待全审计、有效30和Q04全部排除保持，七主项继续。
