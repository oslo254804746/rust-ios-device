# L02 raw H2/RSD operation future payloads · 2026-10-15

H2七个原operation和RSD五个原装箱入口，沿原framer bank对实际size_of_val(F)
在原Box::pin前准入；BoxedOperation先销毁原Pin<Box<F>>再退款。保留既有Box
使nested service/bootstrap future不过度扩大stack；无bank仍是原box，不限制
借用/非Send future，不引入task、独立reader或timer。

H2仍使用原begin_operation/finish/Drop与原deadline，RSD仍使用原logical
progress guard、absolute Clock和stage budgets；配额拒绝在原operation poll前，
可继续使用未消费字节的同一framer。已经progress的原native IO/EOF/取消失效
规则与错误downcast/source不变。没有把新的相对总期限加到bootstrap外层。

新增6项：实际非Unpin future poll/cancel后原地址保持、原析构时fee仍在；
末字节不足原借用future不poll且只析构一次；原typed BrokenPipe错误的非零
payload指针/value经过future销毁后保持；Nonebank借用非Send future；实际
H2分配拒绝后同一framer仍读取原未消费DATA并严格peer EOF；原expired
absolute deadline返回原Elapsed且原future先析构再退费。

既有raw slice/partialframe fixtures改为给真实operationBox足够额度，同时
PAYLOAD_ADMISSION在原4096-byte header对应payload创建前扣到len-1，证明
拒绝的是原raw payload而非新future。partialcancel也明确断言到达32-byte
payload阶段，原framer失效/严格EOF/fullrefund保持。配额池不替换，额外持走
配额只存在于隔离test hook。

首轮XPC147/prepared4/existing18全部通过；strictcore仅新test的一处冗余
matches!断言lint失败，等价改is_err()后按最终冻结源码复验。失败日志保留。
最终冻结590个build-source，XPC147/prepared4/existing18/core131/CLI504
均通过，0失败/2原ignored；strict core/CLI all-feature/all-target Clippy、
CLI build/fmt/diff通过。无默认feature与无tunnel的dproxy+fetchsymbols+restore
all-target编译通过，保留cfg deadcode warnings。readonly manager1，Source/
usbmux trap0；host5 PASS/0 FAIL/22 NOT_RUN，real_device_executed=false。
CLI SHA-256：
`6dc05326e2653b8ac063571377a24410c6cc1fa13d4c18719389061b5d7c0d3c`。最终证据/受测binary在closeout，raw Git
blob/工作区/本地提交由postcommit proof关联。证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-raw-operation-future`。

收费只覆盖实际原Box<F> payload，不宣称TaskCell/JoinSet/Bytesownerheader/
H2 collection backing或outbound编解码对象、initialRSD、watch/oneshot/waiter/
Arc/privateerror/report/allocator/OS或其它副本已经全局收费。128MiB仍非完整
manager heap/RSS上限。原error/strictEOF/native-taskjoins/绝对期限和initial
observation语义继续保持。账本23交付/6缺开发/L02待完整审计、有效30与Q04
全部排除保持，本项不关闭L02。只本地提交，无真实设备发现/配对/安装/恢复、
push/发布/部署；未运行平台/SDK/feature/MSRV/设备保持实际NOT_RUN。

后续审查另发现：原H2 guard对已读header后的typed allocation拒绝尚缺明确
invalidating回归用例；下一独立修复同时覆盖payload与demux准入阶段。此项
对分配前拒绝保持原连接可用的用例不代表header后拒绝路径已完整验证。

随后已以两项真实失败回归修复该边界，见[配额拒绝后失效](tunnel-quota-progress-2026-10-15.md)。
