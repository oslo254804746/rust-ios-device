# L02 SocketStorage 真实 backing · 2026-10-14

userspace不再把可自行增长的owned Vec交给SocketSet。新的SocketSlots持有
真实Vec<SocketStorage<'static>>与其bank permit，只通过同步borrowed view调用
原smoltcp API。添加socket前，按真实slot layout预付replacement capacity；旧
backing仍收费。增长移动完整原SocketStorage后才释放旧allocation/fee，不以
64 client count或估算slot费用代替bytes，也不增加固定连接数量上限。

原SocketStorage包含smoltcp私有Meta、handle和原socket。没有remove后re-add
重置Meta/状态的步骤；空位复用保持原slot index且无需新quota。poll、查询、
add、remove、poll_delay均使用同一真实storage的临时view。实际socket先销毁，
再释放ConnState所持两个1MiB buffers的费用；整个slots Vec最后析构后才退
backing费用。原NativeLease/runtime/parent tasks/连接/取消/绝对期限规则保持。

smoltcp0.11把storage与socket buffer lifetime绑定到同一参数。内部view因此
使用一处lifetime-only unsafe pointer/slice转换。SocketSlots只存owned/static
sockets，HRTB callback对所有scope均有效，不能放入短借用或返回view/其引用；
view不跨await、存储增长或其他别名。转换不分配、不复制或更换node/socket。
维护该边界时必须保持这些约束，不能把任意借用socket接入persistent slots。

4项新增门禁覆盖：actual layout/exact quota/最后owner退款；growth peak/
onebyte拒绝、原buffer地址或完整private Meta/debug/TCP state/endpoint/handles
保持；zero-quota vacant-slot复用；真实metered runtime在socket storage末字节
拒绝后原typed fatal、client/peer严格Ok(0) EOF与task baseline/native joins。
第一次编译仅fixture误用IPv6常量，失败日志保留；改为当前依赖LOOPBACK后
4项和完整core tunnel114通过。Clippy指针转换表达式等价整理后，最终4项
复验与strict core通过。最终完整CLI tunnel504 PASS/0 FAIL/2原ignored、strict
CLI/build/fmt/diff通过，readonly manager1 Source/usbmux trap0通过；host5 PASS/
0 FAIL/22 NOT_RUN，real_device_executed=false。

冻结578个build-source，CLI SHA-256：
`7f82c5a4673d41ec93ee18fd4bdbae3dab040cf871d744e08ef77a6a7fd04e5b`。
实际source/raw Git blob与本地提交由postcommit proof关联；其它feature/SDK/
平台/MSRV/设备范围保持原证据或NOT_RUN。证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-socket-storage`。
首次/round2失败与core114结果保留；最终narrow/strict/fullCLI/build/smoke/
冻结/受测binary/proof在final。只本地提交，无设备操作、push、发布或部署。

本项补齐SocketSet真实slot backing。Tokio channel/private blocks、runtime/
task/report/error/Arc和allocator私有存储、raw H2/XPC、其它copies及原物理刷新
范围仍需合同和交付。128MiB尚非完整manager heap/RSS上限，L02未关闭；
23交付/6缺开发/L02待收口、有效30与Q04全部排除不变。
