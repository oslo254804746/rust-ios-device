# L02 federation future/result storage · 2026-10-16

list_federated与remote_target的普通join_all改为共用owned_join::all。
两个KnownVec分别按真实Slot<F>和F::Output Layout预付完整backing；每个原
future通过exact_backing::<F>(1)先准入再分配，将同一allocation转换为
Pin<Box<F>>，不另建估算胶囊。非零F的array(1)/Box Layout一致，ZST保留
有效对齐dangling pointer；转换之间无可失败步骤，也没有unsafe pin投影。
Box内原future先析构/释放实际payload，再归还独立grant。

所有future body和两组数组均在首次child poll前建立。末个body准入失败会
销毁所有原future并退款，不发HTTP、不触发Source或usbmux。最多32项直接
共用parent Context，不创建collector私有child-waker集合或新任务。保持全并发、
等待全部完成后处理结果、注册顺序、原错误/source和取消时原future析构；
不增加timeout/retry。结果backing费用经IntoIter持续持有，原子结果的独立
grant随原值移动，最后output可继续脱离collector/iterator/manager存活。

取舍是至多32个独立future Box，并在每次wake轮询剩余children；没有据此
声称CPU/RSS更优。子future调用的framework/private waker、HTTP/client/error/
allocator等私有分配不由上述actual payload grant覆盖。

新增10项覆盖真实Layout在allocation hook时已收费、两个数组和最后future
各差一字节时零poll/精确退款、!Unpin原地址、反向完成/注册顺序、错误后继续
等待且原typed IO/Arc指针保留、pending取消和已完成paid output退款、parent
waker实际唤醒、ZST/空输入、高对齐future及原析构先于退款、最后iterator/output
owner。实际两条生产路径各在第二个原future差一字节时拒绝，HTTP/Source/
usbmux调用均为0；没有用估算类型替代生产future类型。

最终专项10PASS；完整tunnel578PASS/0FAIL/2原ignored，上游访问6PASS；
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff及实际只读manager
1项通过，Source/usbmux trap0。host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。617个冻结源码哈希无漂移。core/Cargo本轮未变，
复用fb89530 core诊断7/8和strict证据，不冒充重跑。
CLI SHA-256：`4dcdc467b3ef54e4f94b16a26b3be28331e5049b37bfc8852a4a4f51956b1613`。

完整日志、冻结清单、受测binary、current及提交后rawGit proof位于：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-join\round2`。
round1保留最初8项专项通过记录；最终新增两项后的全部证据在round2。

decoder escaped-string scratch、URI/header/client/registry/credentials/policy/
auth/framework/WS/private IO/error/Arc/allocator/TaskCell、initial公开RSD目录
持久owner和原完整freshness/刷新/复用/重启/manual epoch/选择/prune条件继续。
128MiB仍为已收费自有对象池，不是完整manager heap/RSS上限。L02不关闭，
23已交付/6缺开发/L02待完整审计、有效30及Q04全部排除不变。仅本地提交，
隔离localhost/duplex与只读Source0 smoke，无真实设备、push、发布或部署。
