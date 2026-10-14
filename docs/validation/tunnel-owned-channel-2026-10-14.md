# L02 channel 真实消息 backing · 2026-10-14

manager userspace 的三条runtime channel和两条client channel现在使用实际
fixed VecDeque<Entry<T>>；创建前准入真实capacity、Shared body与Semaphore
body，再分配。消息进入queue后不创建或增长Tokio message blocks。原8192/32
capacity、FIFO和client count保持；无bank构造器/fixtures继续原Tokio路径，不
支付空dummy reservation或把count当bytes。

每个Entry含原value与count permit。发送用既有fair semaphore等待空位，取得
permit后无await提交到实际queue；pending send取消保留队列原FIFO，并释放
尚未发送的原value。recv移出原value后释放count slot，而buffer自己的实际
capacity fee仍随原buffer持有。receiver关闭先标记closed/wake所有阻塞者，
再于锁外销毁原消息和backing；blocked sender返回同一原value。last sender
关闭后receiver先排空已有FIFO再返回None，不因cancelled recv丢失wake。
共享body费用保留到最后endpoint/future owner，不提前当作全部heap已回收。

所有runtime buffer/channel准入均在listener/workers创建前完成，client channel
拒绝保存typed original fatal再stop。原NativeLease/runtime、strict peer Ok(0)
EOF、task/native joins、absolute deadlines和Source规则保持。quota fixtures
同步纳入真实runtime/client channel费用，没有盲目增加总池或绕过目标准入。

新增7项覆盖actual body/backing exact/onebyte退款、last sender body、blocked
send取消与原onebyte逻辑buffer保持4096容量/pointer/费用、sender close drain、
recv取消后wake、receiver close返回同一Box并唤醒send/closed等待、FIFO与
cancelled waiter释放位置、constructor末字节拒绝前worker0及严格EOF。

同时发现旧实际Tx runtime fixture未计SocketStorage backing，可能提前在socket
阶段拒绝。现在补足actual slot费用，并在真实Tx prepare结果处断言已到达、
剩余恰为MTU-1且真实拒绝；保留原typed fatal、repeat joins、EOF和task baseline。
该门禁已通过，原仅ResourceLimit/stop断言没有被用来替代具体阶段证据。

首次编译仅Box integer推断两个fixture错误，日志保留；改为Box<u64>后channel7
与完整core tunnel121通过。test hook类型别名等价整理后，最终channel7复验、
strict core/CLI、完整CLI504 PASS/0 FAIL/2原ignored及build/fmt/diff通过。
readonly manager1 Source/usbmux trap0通过；host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。其它SDK/feature/platform/MSRV/device范围按真实
证据或NOT_RUN保留。

冻结580个build-source，CLI SHA-256：
`a36ef4626bffd1320a7966e852419c6ad4c9958801647b6b42b74318b9ced83f`。
源码/raw Git blob与提交关联由postcommit proof核对。证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-owned-channel`。
首次/round2失败与core121完整日志保留，最终narrow/strict/fullCLI/build/smoke/
冻结/受测binary/proof在final。只本地提交，无设备发现/恢复/配对/安装、push、
发布或部署。

本项移除manager channel私有message-block分配，并收费实际可见payload。
Notify/Semaphore waiter futures、Arc allocator headers、Tokio task/control storage、
native report/error、raw H2/XPC与其它copies/原物理刷新规则继续；128MiB尚非
完整manager heap/RSS上限。L02未关闭，23交付/6缺开发/L02待收口、有效30
与Q04全部排除不变。
