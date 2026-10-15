# L02 H2/RSD sending storage · 2026-10-15

banked H2发送侧不再拼接临时payload/frame Vec：preface后的SETTINGS/window
control、ACK/HEADERS/END使用原future内固定字段；DATA使用inline九字节header
和原payload借用，仍按原窗口/frame上限分块并在原flush边界结束。不创建新
writer/reader/task或timer。Nonebank保留原contiguous build_frame/write_all路径，
原wire顺序、31bit stream/increment mask、error及deadline/guard保持。

banked RSD的三个bootstrap和host handshake，从借用stack grammar先验证长度，
再按真实Layout准入唯一MutableBytes wire buffer。无IndexMap/String/BytesMut
临时树、body/full拼接副本或发送侧Bytes::from_owner header。原uuid、flags、
msg_id、字典字段顺序、NUL/padding、原码值及HEADERS#3顺序保持。Nonebank
继续原XpcMessage/encode_message。原buffer与permit同owner，部分写/取消/
错误时先销毁实际Vec再退款；retained bootstrap buffer仍按原lexical owner
存在期间持费，不把write结束当作尚存buffer已经退款。

新增9项：inline H2 header/settings与原builder逐项对照；真实部分DATA写直接
观察原buffer地址且write/task join后原owner仍收费；header progress后取得的
原typed BrokenPipe pointer/value在framer销毁后保持；原deadline partial send
保留Elapsed/invalidating/严格peer EOF和payload最后owner；借用XPC grammar
全部RSD字段/nested UTF8/内NUL/padding与原encoder bytes对照；exact wire末字节
拒绝/原text pointer/最后owner退款；多UUID原host与bootstrap flags字节对照；
三原bootstrap buffers合计92bytes同时持费至各自drop；真实preface后host编码
拒绝不发送HEADERS，同一framer补额度后发送原identity/wire并严格EOF。

sending筛选9PASS包含8新用例和1原服务用例；另一新exact-wire用例在完整XPC
集执行。不能把筛选数简单当新增数。完整协议回归包含原四mode、Window/
RST/GOAWAY、partialIO/cancel、strictEOF和原错误source，用真实duplex/IPv6
localhost fixtures，未访问物理设备。

最终新增9项、XPC187/prepared4/existing18/core tunnel131/CLI tunnel504
均通过，0失败/2原ignored；strict core/CLI all-feature/all-target Clippy、
CLI build/fmt/diff通过。无默认feature及无tunnel raw组合(dproxy+fetchsymbols+
restore) all-target编译通过，保留实际cfg warnings，不称这两组strict。readonly
manager1，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。冻结601个build-source，当前源码无漂移；CLI SHA-256：
`61f86704199fc64d0ce7a2812dfddf0f84cf44718977915125e65d69556c3a37`。受测binary/完整门禁/sourcefreeze及raw Git
blob/工作区/本地提交proof均在证据目录。证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-sending-storage`。

未据此关闭整个SDK/initialRSD/其它通用XPC编码API：Nonebank兼容路径和其它
route/record/IO copies、receive Bytes私有ownerheader、Tokio TaskCell/JoinSet/
waiter/Arc/watch/oneshot/privateerror/report/allocator与原fullfreshness继续。
128MiB仍为已收费自有对象池，非完整manager heap/RSS。L02不关闭，23交付/
6缺开发/L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备
操作/push/发布/部署；未运行平台/SDK/feature/MSRV/设备保持真实NOT_RUN。
