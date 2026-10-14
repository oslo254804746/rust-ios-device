# L02 native Tx/socket/chunk 真实存储 · 2026-10-14

manager userspace Device 在提供 receive/transmit token 前，先准入真实 Tx queue
backing及整 MTU OwnedBuffer。准入失败先持原 typed cause 到既有 Ledger，再
发出 stop；不提供 token，不调用 closure，不返回空 packet/虚构成功。已取得
token只缩短逻辑len，整 capacity费用随原buffer经过pending/channel/partial
writer直到最后owner。无bank构造器保持原按requested length分配的兼容行为。

OwnedQueue 支付真实 VecDeque capacity。增长前旧backing费用保持，并预付
replacement peak；失败返回原item、保留原queue，成功实际搬移后才释放旧
backing。receive/transmit/pending/local connection/removal queues都接线。原
8192/32 channel与64 client count规则保持，count不替代actual bytes费用。

每次连接先准入真实connection backing，再创建两个1MiB socket buffer。
SocketFees随ConnState持有，SocketSet在connections之前析构；显式remove也
先销毁socket再释放其fees，不因clear/stop/return提前退款。proxy read4096/
receive65536 buffer创建前准入，truncate或partial cursor仍保留原capacity费。
部分socket send通过offset推进原buffer，不复制剩余尾部；write_all未结束前
原buffer和fee仍在writer里。原NativeLease/runtime/joins不更换。

reader/writer/smoltcp的原错误在stop可见前持入原Ledger，worker退出返回typed
work report，OwnedTask识别自身report而不重复记录或制造错误cycle。这个report
不认证全部joins完成。原callerDrop、strict Ok(0) peer EOF、task/nativejoin、
错误downcast/source和绝对deadline规则保留。

新增9项覆盖：token quota拒绝/原fatal/no token、real closure值与整MTU最后
owner、response token拒绝保留原RX、actual queue peak/onebyte、两1MiB socket
buffers与最后释放顺序、onebyte chunk/cursor保持capacity、真实partial duplex
write/fullfee直至join、实际metered runtime拒绝后所有原workers/client stop/join，
以及unmetered exact requested-length兼容。原RX最后token门禁同步计入实际
queue backing，并保持原pointer、EOF和task baseline断言。

外部中断草稿6文件的5个proposal hash已偏离旧manifest；保留旧制品后root逐条
审阅实际源码，另存review940562，修正connection backing准入顺序和unmetered
Tx语义，再生成全文件guard并一次应用。首轮userspace39/core110/activation4
通过，strictcore仅3个lint失败日志保留；整理listener limits并去unused测试
标记后core110/strictcore通过。CLI503 PASS/1 FAIL/2原ignored的census旧断言
预期join前0，与新真实提前持有原16384-byte payload不符；改为观测16384，
保留1 unjoined、不完整测量及join后/history同一payload断言。最终CLI504
PASS/0 FAIL/2原ignored，strictCLI/build/fmt/diff通过。

最终冻结576个build-source。readonly manager1通过，Source/usbmux trap0；
host5 PASS/0 FAIL/22 NOT_RUN，real_device_executed=false。原SDK/feature/MSRV/
其它平台/设备范围保持原证据或NOT_RUN。CLI SHA-256：
`2794ca3e5dcd2db0e861da3318d40b28523659e16dfd66524bb65d5606551945`。
raw Git blob/working source与提交关联由postcommit proof核对。

证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-native-transmit`。
初始、round2失败/成功完整日志保留，最终CLI/strict/build/smoke/hash/binary/
proof在final。实际原/proposal/guard与root修正位于相邻native-transmit-draft
和native-transmit-review-940562。仅本地提交，无设备操作、push、发布或部署。

仍需完整创建合同：smoltcp SocketSet内部slot Vec（public SocketStorage真实
layout可知但原API不暴露capacity）、Tokio channel/private blocks、runtime/task/
report/error/Arc/allocator私有存储，raw H2/XPC及其它copies和物理刷新规则。
SocketSet不能通过拆出再新建socket重置其Meta/handle。128MiB尚非完整manager
heap/RSS上限；L02不关闭，23交付/6缺开发/L02待收口、有效30和Q04全部
排除保持。
