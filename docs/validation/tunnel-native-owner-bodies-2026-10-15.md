# L02 native owner body/backing · 2026-10-15

metered userspace在创建原workers前，先按真实size_of准入并创建Ledger与
SharedChildTaskSet的应用body，再准入真实四项OwnedTask Vec backing。原
无bank/kernel构造器保持兼容；本项不宣称其task/runtime私有存储已全部计量。
同一bank沿原activation/reservation进入，原runtime、NativeLease、任务handles
和join账本不更换。body fee跟随最后共享应用owner；child drain或task joins
完成不表示仍存的owner body/backing已销毁。

TaskHandles把实际Vec与permit封在同一owner内，先销毁Vec再退款，包含
constructor/listener失败路径。四个parent handles仍为原实例，初始化不增长
backing。quota不足在listener/workers创建前返回typed ResourceLimit，不生成
假success，不丢已经取得的原native IO或join错误。

native failure snapshot的cleanup改为实际inline Option<Cause>字段，覆盖全部
既有retained slots与本次native expiry，不再创建cleanup Vec。Debug也直接遍历
分类，不再collect临时Vec。primary/source、cleanup顺序、report identity、
不可变历史、pending joins/超时及原cause的Arc/pointer保持。report body、IO/
Arc/allocator内部创建仍需后续完整合同，不能因移除Vec而称其已全部收费。

新增5项覆盖：Ledger body exact/onebyte/最后共享owner；真实child spawn/drain
后直到最后set owner保持费用；零剩余额度下全部10个原cause及native expiry
保留于inline snapshot，Ledger释放后原pointer不变；真实task-handle backing
末字节拒绝与原handles join后持费至Vec销毁；constructor末字节失败前worker0、
全部临时owner退款与严格peer Ok(0) EOF。原Tx实际MTU准入hook/fatal/repeat
joins/client与peer严格EOF、task baseline断言保持。

首轮新增5项通过、完整core125 PASS/1 FAIL：旧Tx fixture在仍持有原native
handle时要求全额退款，而新owner body/backing实际仍存在。现按真实
runtime_owner_bytes保持费用，两个原handle均释放后断言全额退款。失败日志
保留。最终完整core tunnel126、CLI tunnel504 PASS，0失败/2原ignored；
strict core/CLI all-target/all-feature Clippy、CLI build/fmt/diff均通过。
readonly manager1 Source/usbmux trap0通过，host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。其它SDK/feature/平台/MSRV/物理设备按真实范围
保持原证据或NOT_RUN。

冻结582个build-source，CLI SHA-256：
`88fe9a211b2dc59020c75f11c81deaaabdc6e2cb5558327bb90b33580abf10f6`。
源码/raw Git blob与本地提交由postcommit proof关联。证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-native-owner-body`。
新增5项/原失败在父目录，最终完整core/CLI/strict/build/smoke/冻结/受测binary/
proof在round2。只本地提交，无设备发现/恢复/配对/安装、push、发布或部署。

剩余创建合同包括Tokio task/control/future、JoinSet私有backing、Notify/Semaphore
waiters、report/error/IO/Arc allocator内部、raw H2/XPC和其它copies及原物理刷新
范围。128MiB仍是已收费自有对象池，不是完整manager heap/RSS上限。L02
未关闭，23交付/6缺开发/L02待收口、有效30与Q04全部排除保持。
