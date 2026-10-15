# L02 original vectored H2 sending · 2026-10-15

banked发送直接借用原payload并将inline header与payload两个IoSlice一起交给
底层AsyncWrite。H2Stream转发原poll_write_vectored/is_write_vectored，按原
实际n累计header/payload offset，处理partial或默认first-nonempty fallback。
不分配frame Vec或IoSlice Vec，不引入task/reader/timer。NoBank保留原contiguous
frame路径。原payload source pointer、最后owner费用、window debit/flush/absolute
deadline不变；支持scatter/gather的TCP可收到两个slices，不保证OS每次写完。

progress/failed标志在vectored路径与原scalar规则一致，native Err原样返回。
非空buffers的Ok(0)明确产生与锁定Tokio WriteAll实现相同的标准
io::ErrorKind::WriteZero.into()，不返回假成功。实现依据已查看本机锁定Tokio
src/io/util/write_all.rs；未用字符串推断分类。header部分写后错误/超时仍使
原framer失效并保留原native cause/Elapsed；无vectored支持的writer回退仍通过
前项实际partial-write/native-error/deadline用例。

新增3项：实际IPv4 localhost TCP确认原生vectored能力、原buffer地址的两个
slice一起提供、原wire/费用直到payload最后owner、严格peer EOF；真正部分
header后原typed BrokenPipe pointer/value在framer销毁后保持且严格EOF；非空
vectored Ok(0)返回原标准WriteZero并失效，零wire/strictEOF/owner退款。全部
仅localhost/duplex，无物理设备。完整XPC集复验既有四mode、banked/no-bank
发送、RST/GOAWAY、window、cancel、错误归属和strictEOF。

最终新增3项、XPC190/prepared4/existing18/core tunnel131/CLI freshness13
全部通过；strict core/CLI all-feature/all-target Clippy、CLI build/fmt/diff通过。
无默认feature和无tunnel raw组合all-target编译通过，保留实际cfg warnings。
readonly manager1，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。冻结602个build-source，当前源码无漂移；CLI SHA-256：
`70606b86b74d5520392d5a2bab7369a7da2f562e36a1113e8100919a6c56f02d`。受测binary/完整门禁/sourcefreeze及raw Git
blob/工作区/本地提交proof均在证据目录。本项复用立即前一提交9b703c3上的完整CLI504/0FAIL/2原ignored
证据；精确新源码运行协议相关完整XPC及CLI freshness，不将前一提交的504
冒充本次重跑。精确新源码重新strictcoreCLI/build/fmt/diff和只读Source0 smoke。
证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-vectored-sending`。

此项只保留原生借用/发送能力，不声称已经完成全部SDK/initialRSD/其它IO
copy、Bytes ownerheader、Tokio TaskCell/JoinSet/waiter/Arc/privateerror/report/
allocator或原fullfreshness。128MiB非完整manager heap/RSS，L02未关闭，
23交付/6缺开发/L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实
设备发现/配对/安装/恢复/push/发布/部署；未测平台/SDK/feature/MSRV/设备保持
真实NOT_RUN。
