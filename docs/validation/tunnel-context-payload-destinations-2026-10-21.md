# L02 pre-Source actual context payload destinations · 2026-10-21

四类原context把causes/receipts与费用字段放在opaque anyhow对象内部，字段Drop
早于外壳分配结束。此项保留原typed wrapper SourceContext、FailureContext、
SourceFailuresContext和ScanCleanupFailure，独立causes/receipts改放在真实的
owned_box::Owner<Payload>中，费用位于该实际Box外。真实payload先析构/释放，
随后才退其整份费用。wrapper名称、原primary.context类型、source与secondary/
completion/receipts accessor及byref/byvalue downcast保持；只移动原anyhow/
receipt handle，没有重建、格式化或替换深层原IO。

Outcomes.begin先原子取得整个SourcePayload、FailurePayload、CleanupPayload、
History和Abandoned总grant，再产生五份实际backing。Session保存独立的Source/
Failure typed目的地与Cleanup目的地。终态处理仅选中place，未用分支先释放
自己的真实存储再退款；删除旧max预留后按小分支退差额的做法。两分支实际
各有自己的layout，不会保留仍占用的物理尾部却提前退费。原scan slot与原
History/Abandoned根仍按原顺序持有错误和历史，Source后无需新的受控payload
分配或字节准入。

WriteBatch在Source窗口前整笔取得WriteOwner与真实ReceiptPayload费用，并
分配ReceiptPayload空目的地。WriteState保存它，finish_scan/abandon只把原
KnownVec<Receipt>移入原目的地；成功/空receipts释放unused目的地。WriteState
和WriteOwner的实际尺寸自然进入原公式，原intents/receipts数组backing与
root地址/费用保留。Cleanup消费真实typed destination构造原wrapper，并继续
source委托原primary。cfgtest外来FailureContext保留原外来cause，不伪造准入。

owned_box生产安全算法不变：真实capacity-one原分配由Box持有、费用在Box外；
测试专用零尺寸AfterFree字段位于实际Vec/Box之后、费用之前，观察unused、
正常Drop、原T析构panic及into_inner的真实free-before-credit。into_inner仍
先释放原Box body/费用再返回原值，原返回值与字段语义保持。

新增4项必要测试并增强原观察器：scan五个真实backing的layout/原目的地地址、
首个alloc前整笔已借记；原WriteBatch完整前缀2/5份实际backing、ReceiptPayload
和WriteOwner首次分配前整笔借记；闭池零额度下Source后禁止任何新的受控
backing，Source/Failure/Cleanup/Receipt四类payload地址在typed byvalue提取
前后保持；unused分支的after-free借记点、最后payload Box释放后才退款；
WriteOwner finish/abandon保留原completion IO、receipt根和深层IO地址。原
History/Abandoned/root/backing、byref/foreign context、零配额/caller Drop、
实际native joins与严格EOF断言保留，没有扩大固定quota或忽略/删除测试。
私有cleanup方法改名cleanup_destination以匹配它现在返回的实际目的地。

第一轮前6组专项全部通过，strict CLI发现tunnel.rs原OwnedSemaphorePermit
import在费用字段移出后已不再使用；整组终态后只删除该遗留import，无告警
抑制或其它生产算法变更。保留失败日志，重新冻结后先跑strict CLI再跑全部
13个门禁。第二轮全部通过：owned Box9、scan19、bookkeeping9、error census9、native
metadata7及freshness17；完整CLI1760 PASS /
0 FAIL / 13原ignored，strict CLI all-features/all-targets Clippy -D warnings、
build/fmt/diff、实际只读manager1（Source/usbmux trap0）、host5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。全部Cargo及cargo build由主控串行执行；
子Agent修改和静态自审，主控复核、格式化与冻结。Core/Cargo未改，复用55f21fe
prepared Core8/owned真实service restart证据，不声称本轮重跑Core/Miri/其它
平台/真实设备。684冻结源码无漂移，9处源码差异；实际host报告与归档受测
CLI SHA-256一致：`bf7fb32513a611d00052f881282c5401c549f1f7d6ba22d0240e7fc703f77d75`。
日志、source delta、verified-ios.exe、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-context-payload-destinations\round2`。

本项准入的是新增实际Payload Box。原typed wrapper自身及其Owner inline字段
仍位于opaque anyhow分配内，其完整layout/释放计费尚未解决，仍保留为原范围
未完成项；没有将它们豁免，也不称Anyhow外壳已全计费。publication body、
freshness两个report、其它marker/private headers、SDK/发现、HTTP/WS/配置/
凭据、框架IO/task/allocator及endpoint恢复继续。128MiB仍是已准入已知对象
池，非完整heap/RSS。L02未关闭；23交付/6缺开发/L02待全审计、有效30与Q04
全部排除不变。仅本地提交，无真实设备、push或发布。
