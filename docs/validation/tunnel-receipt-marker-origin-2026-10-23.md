# L02 receipt marker origins through final Weak · 2026-10-23

FailureReceipt原先在bank B支付完整共享根，但其中Ticket可来自另一
producer A。原native Entry/proof修复不覆盖该独立receipt边。本项在
实际receipt Source前准入时，为Ticket捕获的各foreign producer支付
同一实际receipt完整分配费用，保留到最后一个Weak实际物理free之后。
这不完成所有retainer或physical marker arena的证明。

Ticket::origin_grants读取其原captured global、device和optional desired
三个ControlIdentity真实出生bank，不从当前Cell版本、UDID或调用者标签
猜测。数组3恰为这些字段数，非producer/epoch/arena容量限制。同bank
已有原B完整分配grant，重复producer去重；foreign配额不足/closed在
原selector/capsule分配和Source前拒绝，暂存所有A/B费用完整回退。
funded独立A/B继续支持；Clone/Weak upgrade/late capture/清理没有新
quota请求或新fallback。

Receipt直接使用原shared_weak_allocation::Strong<FailureReceipt,
TicketOriginGrants>。实际声明的Allocation<T,E>完整布局包括原strong/
weak计数、B grant、额外origin grant数组、原值及padding；body_bytes
直接来自这个原真实类型。原B body grant和origin grants都在分配器的
Fees中，独立于T值，不能放在FailureReceipt值里于last strong提前退费。
T仍在原地址last strong析构，最后Weak归零才free同一个原allocation，
之后才退B及origin费用。原unsafe分配/clone/weak/Drop实现逐字保持。

只给该通用Strong补上原wrapper已有的Debug/Display转发，仍仅格式化
原T值，不输出fee/bank。原单bank shared_body::ptr_eq生产已无调用，
标为cfg(test)保留测试用法，不用allow(dead_code)跳过严格检查。原
FailureReceipt内容、selector buffers、Original IO/history、SourceFailure
wrapper、completion及Ledger retry计数/时钟/清理正文保持；只有真实
receipt类型/根布局与pre-Source origin准入改变。

新增3项实际离线验证：

- A真实N−1及足额但closed拒绝，零selector/capsule受控分配；B原
  slot/version/temp grants全退，A余额和原Ticket保持，未调用Source。
- optional captured desired-version来自另一真实producer C：C N−1
  拒绝退回已暂存A，B全退；funded后3字段只产生A/C两笔去重整费。
  selector/root受控分配观察点确认两origin都已持费。last strong结束
  后Weak不能upgrade但A/C/B完整body费均未退；最终Weak物理free
  回调仍观察三笔费用，之后分别准确退款。构造器对stale观察Ticket
  仍按其真实captured marker计费，不替换为当前版本或假装授权通过。
- 实际Ledger A/B adapter，预付receipt后两bank held至0并关闭，再
  clear/close和late Source错误捕获，原Deep对象及更深IO字段地址、
  PermissionDenied类别和Abandoned状态保持。所有原selector/Ticket/
  Ledger字段结束后，A和B仍各欠准确body_bytes直到最后Weak物理
  free观察点；最后完全退款。无真实设备/SDK/native操作。

原receipt/body/weak/parallel snapshots、native joins、retry max/backoff/
总deadline、Source IO根、clear/stop及实际manager脚本回归保留。没有
把Receipt clone算作新根或新费用。Episode仍inline于Removed Box，
Removed::into_inner还会释放Box后保留Episode/Ticket，必须独立证明其
真正值寿命；本项未修改或冒称覆盖这个其它retainer边。

首次全部test-target check通过。focused1仅strict Clippy报告旧shared_body
ptr_eq在receipt迁移后生产未使用，按实际只测试用途加cfg(test)；随后
focused2七门禁全部通过。当前round1冻结后32门禁全部通过，所有
Cargo/build由主控串行执行：

- receipt-origin：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1863 filtered out; finished in 0.00s
- reconnect：test result: ok. 50 passed; 0 failed; 0 ignored; 0 measured; 1816 filtered out; finished in 6.82s
- shared-weak：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1862 filtered out; finished in 0.00s
- origin-native：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1862 filtered out; finished in 0.01s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1819 filtered out; finished in 0.18s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1837 filtered out; finished in 6.71s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1856 filtered out; finished in 0.00s
- manual-success：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1853 filtered out; finished in 0.71s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1849 filtered out; finished in 0.04s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1857 filtered out; finished in 0.01s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1853 filtered out; finished in 0.11s
- error-payload：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1857 filtered out; finished in 0.02s
- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out; finished in 0.00s
- manager-owner：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1862 filtered out; finished in 0.00s
- native-retirement：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1756 filtered out; finished in 43.65s
- intent-route：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1855 filtered out; finished in 1.54s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1861 filtered out; finished in 0.05s
- scan-frame：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1861 filtered out; finished in 0.14s
- ensure-frame：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1860 filtered out; finished in 0.08s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1850 filtered out; finished in 0.04s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1858 filtered out; finished in 0.04s
- future-storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1862 filtered out; finished in 0.00s
- publication-retain：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1863 filtered out; finished in 0.06s
- cli-full：test result: ok. 1853 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 245.65s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。712冻结源码无漂移，6处CLI差异（含1新增
测试文件），Core/Cargo.lock与d3/ec25全部hash保持，旧Core24证据沿用，
未声称本轮重跑Core。原lifecycle除新helper外、原receipt capture/history/
IO/completion、原分配器全部unsafe/weak/Drop协议和原existing tests都有
主控源码相等guard。主控审核，不冒称独立Agent实现批准。
实际host与归档verified-ios.exe SHA-256：
f3d81d3ff65e0ece2846a7e912d708717df7a300e4e37f1086e6fee4c88fff3c
完整命令/log/result、source guard/delta/current/提交后rawGit proof及二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-receipt-marker-origin\round1

actual receipt完整共享分配现在保持正确origin费用，底层control identity
std Arc分配本身未因此变成paid arena。Episode/Removed、其它Ticket/
publication/frame/opaque/Weak keepers、总容量authority、控制峰值以及
public/Core/SDK/legacy/HTTP/framework/task/allocator/waker/waiter/transport
和全部mode endpoint恢复/native exposure仍是原scope。无Miri/其它平台/
真实设备验证。128MiB仍是已知对象准入池，非整个heap/RSS。23交付/
6缺开发/L02待全审计、有效30/Q04全部排除不变，原七主项目标继续。
