# L02 native marker producer-bank admission · 2026-10-23

bb4be93已执行的有限census表明，独立B Registry可保留A的旧stamp，而
A原有费用全退。原Arc<()>自身没有真实producer-bank来源，不能靠调用者
另传标签假定所有keeper都支付A。本项接入真实native Entry与独立
RefreshProof的origin准入，不选择physical arena容量，也不完成L02。

私有ControlIdentity在唯一构造点实际创建原Arc<()>，同时保存实际
producer bank；不接受既有Arc来重新贴标签，字段私有、无raw/Deref
出口。Clone及测试Weak upgrade保留同一个实际Arc身份及bank；比较仍
调用其原Arc::ptr_eq，Debug仍仅输出原()。Lifecycle/Cell各原birth
位置和原bank保持，含global/device/scan/desired-version；standalone
由原Registry bank直接出生两身份。clear/stop/lazy reset仍无新配额取得，
可在closed0继续。底层std Arc控制分配仍未由本wrapper支付，Weak
观察语义保留不等于完成Weak实际分配计费，physical arena仍是原任务。

Registry在原sealed/capacity/issued overflow检查及原Incarnation准备
后，暂存新foreign origin A的keeper费用，数量精确为原实际
_slots_storage.num_permits，即B整份fixed slots Vec backing。same-bank
已由原B backing grant支付，跳过重复mirror；有足够真实配额的foreign
继续成功，只有A自身配额不足/closed才在Source前拒绝新keeper。
这不是按bank标签一律拒绝。已付origin的B后续重用slot不重复扣A，
退休/clear/stop/drain/Drop不取得新origin配额。

历史origin目录由B支付实际Vec backing，按2/4/...实际增长，不用当前
slot数充当历史producer上限，也不加任意cap。至多两个incoming grant
来自OwnershipStamp恰好两个字段；这不是arena/producer容量上限。
原目录、所有新A grants、原Incarnation和新B backing全部成功后才
commit，再消费原ordinal及slot。拒绝退回所有暂存费用，不留下目录
记录或改变旧slot/history/counter。增长移动原A grants不退款，旧B Vec
先实际free再退旧B backing fee，增长期间两份backing均已支付。

目录是Registry最后字段，晚于State slots Vec、原report根和原slots
grant销毁。因此一个slot清空不会退整份A mirror，原整个B Vec实际
free后才退。cfg structural_bytes包含实际目录backing；cfg rebind
只允许原空目录，不能丢弃仍需保持的历史A费用。State→origins锁顺序
一致，没有新增异步等待、任务、时钟、重试、unsafe或SDK路径。

只给B目录mirror仍不够：RefreshProof可在B slot移除/重用后独立保留
旧stamp，且其原_storage可来自第三bank C。现在原capture验证和C的
逻辑sizeof<Self> grant之后，各foreign producer另持同大小origin
grant；same-bank及重复origin不重复扣费。这些grants是proof最后字段，
在stamp实际Drop后才退款。原registry→key→stamp→handle→metadata
字段析构顺序保持：B的mirror先退时，proof自身A费用仍覆盖活着的stamp。
它延续原_storage的正逻辑配额语义，不是新增一个独立proof heap
allocation；实际父F/backing费用另行支付，不把逻辑fee冒称独立分配。
拒绝退回C及部分A grants，保留旧active handle/metadata/catalogue，
原capture仍发生在Source之前，没有新fallback或延长deadline。

新增及演进的实际离线证据：

- opaque Identity原value/bank clone、不同实际nonce、closed0出生和Weak
  upgrade控制语义通过；没有用bank标签或数字generation代替实际身份。
- 原四B census改为先给A足够实际keeper费用，再在A closed0时完成最后
  clear/lazy reset。八个实际旧身份strong1且28对不同；producer Drop
  只退其原根，slot release不退whole mirror。B final free的三个真实
  Shared回调含两个在slots Vec销毁之后的report根，A fee始终仍持有，
  之后准确退款。B的目录与Incarnation费用分别精确核对。
- origin N−1/closed及B目录N−1拒绝，所有暂存A/B/Incarnation费用恢复，
  issued0、slots0、原目录未变。原raw census的A全退是旧未计费现状，
  不是新合同；未把它隐藏为不再测试的foreign豁免。
- 三个历史producer在一个当前slot上反复使用，目录正确容纳超过slot
  数的origins；origin已付后closed0仍可重用，直到B whole free才退款。
- 目录2→4增长的N−1回滚及随后两次真实分配观察，原A fees不退款或
  再扣，旧/新B backing同时付费。mixed两origin第二边失败退回第一边，
  funded成功两边皆扣；原slot capacity拒绝优先且零新allocation。
- 实际foreign proof测试分别用同一及两个不同producer banks，每种
  情况依次创建两个duplex userspace native并重用同一B slot。proof有
  独立A费用，第一边及第二边A拒绝均退回C/已暂存A、保持原active
  handle、metadata pointers和原proof.check。两次原native drain全部
  joins、严格peer.read==Ok(0)及任务基线通过。
- proof留到slot移除和外部B owner结束以后；第一个proof只退自身费，
  最后proof的四个真实free回调核对B mirror与Key次序，Key free时弱
  观察仍能upgrade原stamp，而proof自身origin费用仍持有。最终所有
  A/C/publication/B原费用全退。没有轮询真实CLI NativeSource/SDK。

compile1生产check首次通过；测试接口迁移compile2/3分别报告旧Arc调用
67/14处类型问题，已逐处迁移、修正测试中四个误转的bank clone。
compile4全部test-target check通过。focused1八门禁先全通过；随后把
实际proof用例扩展为同/异producer和第二边拒绝，本round1冻结后全部
30门禁通过。所有Cargo/build由主控串行执行：

- identity：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1862 filtered out; finished in 0.00s
- origin-census：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1860 filtered out; finished in 0.00s
- origin-native：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out; finished in 0.01s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1816 filtered out; finished in 0.19s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1834 filtered out; finished in 6.69s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1853 filtered out; finished in 0.00s
- manual-success：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1850 filtered out; finished in 0.73s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1846 filtered out; finished in 0.05s
- reconnect：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1816 filtered out; finished in 6.81s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1854 filtered out; finished in 0.01s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1850 filtered out; finished in 0.10s
- error-payload：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1854 filtered out; finished in 0.02s
- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1856 filtered out; finished in 0.00s
- manager-owner：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out; finished in 0.00s
- native-retirement：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1753 filtered out; finished in 43.60s
- intent-route：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1852 filtered out; finished in 1.93s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1858 filtered out; finished in 0.82s
- scan-frame：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1858 filtered out; finished in 0.14s
- ensure-frame：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1857 filtered out; finished in 0.08s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1847 filtered out; finished in 0.04s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1855 filtered out; finished in 0.04s
- future-storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out; finished in 0.00s
- publication-retain：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1860 filtered out; finished in 0.06s
- cli-full：test result: ok. 1850 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 246.78s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff，
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。711冻结源码无漂移，30处CLI差异（12生产/
18测试，3新增）；Core/Cargo.lock与04da8/旧ec25保持，Core24既有证据
沿用而未称本轮重跑。原控制切换/时钟、Ownership、parent其它正文、
native selection/joins/Drop/handle/metadata、原proof Drop及swap/causes
有主控源码比对，忽略rustfmt空白/调用尾逗号和新增import位置。
主控审查与实际门禁是当前证据，没有独立Agent实现批准；原设计外部
报告不是通过验证的实现。实际host与归档verified-ios.exe SHA-256：
8548d30c03c8ffe24b8e0de60d2d4f5f7c13e75d5d3973092f0dd08b5c88c76c
命令/log/result、source delta/guard/frozen/提交后rawGit proof及二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-marker-origin\round1

该项仅处理真实出生bank与已知native Entry/proof stamp keeper边，未
证明所有Ticket/receipt/Removed/publication/frame/opaque/Weak keeper
均支付origin，也未选arena cap。std Arc/byte-bank根、physical arena
整backing寿命、不可变TOTAL authority、控制峰值、SDK/public Core/
legacy/HTTP/framework/task/allocator/waker/waiter/transport与全部mode
endpoint恢复/native exposure仍是原scope，不以wrapper/private边界豁免。
128MiB仍为已知对象准入池，非整个heap/RSS；无真实设备/Miri/其它平台
验证。23交付/6缺开发/L02待全审计、有效30/Q04全部排除保持，原七主项
目标继续。
