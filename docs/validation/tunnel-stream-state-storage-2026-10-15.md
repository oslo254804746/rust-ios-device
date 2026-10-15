# L02 H2 stream state actual storage · 2026-10-15

metered H2的stream buffer/reset两张表、local-open/peer-open/closed/local-finished
四组membership，以及unknown EOF/reset arrival两队列，均沿原bank按真实
Layout在创建前准入。复用显式key/value backing并用u32 arrival Vec保持原顺序；
old/new增长peak同时收费，clear/remove后的真实backing仍存则持费至owner销毁。
Nonebank保持std HashMap/HashSet/VecDeque，原MAX_UNKNOWN/MAX_CLOSED限制不改。

local stream全部metadata在HEADERS前准入；local-finished在END前准入；peer
membership在最高stream ID推进前准入。END/reset先准备所需record/arrival
backing，成功后才移除旧buffer、淘汰最旧unknown墓碑、提交新状态；满队列
替换复用已付空位，不因零额度另行分配。已知1/3和local stream仍固定保留，
最高peer ID单调，不因墓碑淘汰而允许复用。已成功预备但随后其它准入失败的
实际backing保持收费，随原framer释放；不把仍存backing假称已经退款。

stream ready/unfinished选择去掉collect/sort临时Vec，直接逐个选择下一个最小
ID。原升序和早期malformed header/body错误优先级保持，既有完整消息先于
其它stream的不完整EOF。原FrameBytes/RawBuffer及其fee不复制、不重建owner，
删除record或framer后返回slice仍持原payload费。

新增PeerStateAdmission在新metadata拒绝路径中保存已观察stream ID及reset
code或END事实，并保留原quota IO source。该路径在原dispatch尚未成功记录
peer状态时发生，返回ResourceLimit；不会伪称已成功记录reset或静默丢弃peer
事实。正常reset仍是原stream-local protocol错误及原code，GOAWAY/Native IO/
strict EOF和原deadline/guard不变。private IO/error boxing/allocator、该错误
wrapper body等完整创建合同仍是MISSING，未因保留事实而宣称所有错误内存已
纳入128MiB；已收费对象池仍非完整manager heap/RSS。

新增12项：membership原Set 3000步对照与零quota替换；arrival增长末字节拒绝/
顺序/实际backing费用；满32项零quota连续淘汰；原payload slice离开record与
framer后持费；真实local buffer记录拒绝在HEADERS前且同一连接可重试；真实
finish membership拒绝不发END并保留原buffer指针；peer准入拒绝不推进原
highwater；真实wire EOF固定已知/local状态、unknown最旧淘汰与不复用；真实
RST同样保持原code和到达顺序；RST记录失败保留事实和原quota pointer到
framer销毁后；END arrival拒绝的事实/已预备backing费用/退款；零quota原升序
ready/unfinished/error选择。全配置helper统一production与已有actual quota/
writer fixtures，旧payload/queue/window/closed费用的最后owner断言保持。

最终新增12项、XPC178/prepared4/existing18/core tunnel131/CLI tunnel504
均通过，0失败/2原ignored；strict core/CLI all-feature/all-target Clippy、
CLI build/fmt/diff通过。无默认feature及无tunnel raw组合(dproxy+fetchsymbols+
restore) all-target编译通过，保留实际cfg deadcode warnings，不称这两组strict。
readonly manager1，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。冻结596个build-source，当前源码无漂移；CLI SHA-256：
`b789d3e181126baa6207bb1e3737e06b53b151a170c470c318f6ea02e70d0d4e`。受测binary/完整门禁/sourcefreeze与raw Git
blob/工作区/本地提交proof均在证据目录。证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-stream-state-storage`。

其它outbound H2/XPC encoding/bootstrap临时对象、initialRSD/SDK copies、Bytes
ownerheader、Tokio TaskCell/JoinSet/waiter/Arc/watch/oneshot/privateerror/report/
allocator与原fullfreshness继续。L02不关闭，23交付/6缺开发/L02待全审计、
有效30与Q04全部排除保持。仅本地提交，无真实设备发现/配对/安装/恢复或
push/发布/部署；未运行平台/SDK/feature/MSRV/设备保持实际NOT_RUN。
