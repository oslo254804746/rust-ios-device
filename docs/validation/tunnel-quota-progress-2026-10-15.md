# L02 H2 admission failure after wire progress · 2026-10-15

实际回归确认：H2已消费frame header后raw payload分配被拒绝，或已读完整
payload但demux增长被拒绝，原finish仅检查底层socket failed标志；配额拒绝
不设置此标志，framer误报可继续使用。两个隔离duplex用例先按实际payload/
demux creation hook把原bank扣至len-1，原代码均在is_usable断言失败；red日志
保留，非仅静态推测。

finish现在对stream.progressed且原typed IO错误分类为ResourceLimit的结果
明确invalidate，清掉incomplete parser/demux/pending状态，再返回同一原错误。
原socket失败、超时/cancel guard、stream-local reset/GOAWAY规则保持。尚未
poll/read/write的operationBox准入拒绝不触发失效：原连接与未读DATA仍可使用，
此前真实用例与本次完整XPC集一起通过。

两项新回归覆盖raw payload和demux准入阶段；修复后均确认原ResourceLimit
和其原source指针在后续拒绝IO后保持，framer不再可用，原bank在数据/fees释放
后全额退款、同一peer最终严格Ok(0) EOF。未新增reader、task或timer，不改变
绝对期限，不用字符串内容分类。已取得错误先保存在result内再清理原状态。

最终冻结590个build-source：XPC149/prepared4/existing18/core131、CLI相关
freshness13全部通过；strictcore/CLI all-feature/all-target Clippy、CLI build/
fmt/diff及无默认feature/无tunnel raw组合编译均通过。readonly manager1，
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
CLI SHA-256：
`bd6ce53f7514c44b29fbeae9ec7e3c035f935a73c68d99b79dbec91049e1b018`。raw source当前无漂移，Git blob/本地提交
以postcommit proof关联。本项复用立即前一提交4ec73e7上的完整CLI504/0FAIL/2ignored
证据；在本次精确新源码上运行与目录/lease归属相关的freshness13及完整XPC，
不会把前一提交的504冒充本次新源码运行。新源码重新strictcoreCLI/build/
fmt/diff和只读Source0manager/host smoke。测试编排曾误把CLI参数分隔符位置
当filter替换，Cargo在执行测试前拒绝命令；原日志保留，正确尾段单独落tail。

证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-quota-progress`。
red保存原2FAIL，green保存生产修复后原冻结源与core门禁，green/tail保存修正
编排后的CLI/构建/受测binary/hash/commit证据。原错误、strictEOF、task/native
joins和原freshness规则继续保持；128MiB仍非完整manager heap/RSS。

H2collection/outbound/initialRSD/privateBytes/Tokio/Arc/waiter/error/report等
完整创建合同及原全局freshness审计继续。L02未关闭，23交付/6缺开发/L02待
完整审计、有效30与Q04全部排除保持。只本地提交，无真实设备操作/push/发布/
部署；其它未运行平台/SDK/feature/MSRV/物理设备按真实范围保持NOT_RUN。
