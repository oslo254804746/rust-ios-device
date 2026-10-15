# L02 native task future payloads · 2026-10-15

metered userspace的四个parent和每个本地代理child先按实际size_of_val(F)
准入，再创建Pin<Box<F>>。费用与原boxed future封装为同一owner，字段顺序
保证原future及其实际allocation先析构再退款，覆盖正常退出、取消与unwind。
原四个parent payload全部准入后才spawn第一个worker；第四个不足时原stream、
listener和前三个prepared owner全部释放，不产生部分workers。

本地代理child在原listener内、原JoinSet spawn前准入；失败先把原typed
BufferAdmission::Exhausted记录为Listener fatal，再发stop，原父子join账本、
重复shutdown和严格EOF仍使用原native owners。Nonebank保持原future直接
进入Tokio spawn/JoinSet，不新增Box或配额限制。原stop时序、cause/downcast、
任务身份与deadline逻辑保持。

本项收费对象是显式Box<F>的实际payload allocation；未把Tokio TaskCell/
控制/结果存储、JoinSet私有节点、Arc头/Notify/Semaphore/waiters、watch/
oneshot/IO/report/allocator内部、其它副本或raw H2/XPC视为已完成收费。
128MiB仍为已收费自有对象池，不是整个manager heap/RSS上限。

新增5项：实际pinned future poll后cancel/join、原地址/析构前仍持费；
末字节不足时原capture恰好析构一次；第四个真实parent future不足时worker0、
前三个退款与peer严格Ok(0)；原child JoinSet cancel/drain持费到原future析构；
真实listener的第五次future准入在child分配点不足，原typed fatal、两次shutdown/
完整join、peer与client严格Ok(0)、task baseline及最终全额退款。

既有Tx/socket runtime fixture改为在实际目标allocation前持走bank的多余额度，
分别剩真实queue backing + MTU-1和真实SocketStorage请求-1。目标阶段hook、
原typed cause/joins/EOF及仍存native owner收费断言保持；不再依赖已过时的
constructor prefix大小，也不会在新future收费处提前失败而误报覆盖。

最终core tunnel131、CLI tunnel504通过，0失败/2原ignored；strict core/CLI
all-target/all-feature Clippy、CLI build/fmt/diff均通过。readonly manager1
Source/usbmux trap0通过，host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。初版parent-only core129通过；扩展child后首轮
因测试泛型Ok类型未注明而编译失败，日志保留。代码修复后按新冻结源码完整复验。最终584个build-source，当前源码hash
与冻结一致；CLI SHA-256：
`2c5bd4d44bb8335b513c7404410fb19ea610491b8ada63ba9b2991dcd480a680`。受测binary及最终门禁在final2；raw Git blob/
本地提交由postcommit proof关联。
证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-parent-future`。

只本地提交，无设备发现/恢复/配对/安装或push/发布/部署。其它feature、平台、
MSRV与物理设备维持实际证据或NOT_RUN。L02未关闭，23交付/6缺开发/
L02待收口、有效30与Q04全部排除不变。
