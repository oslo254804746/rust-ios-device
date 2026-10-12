# L02 snapshot/publication 共享已知字节额度 · 2026-10-12

在36febe2快照容量准入上，将四个Source消费点的已知raw snapshot成本与已发布
元数据纳入一个128MiB共享Semaphore。原64MiB publication子额度保留；两个32MiB
snapshot有扫描容量，原发布额度满时不会仅因该子额度阻断正常小型快照刷新。
字节数由已通过本轮capacity/checked validator的原Vec/String容量计算，不格式化
元数据；准入是同步try-acquire，不持全局锁等待网络或新建等待/重试任务。

USB raw charge覆盖paired discovery、candidate与后续状态等待；paired只有整批
规划通过才继续持charge，拒绝立即释放其原targets与charge，不挤占USB后续处理。
caller取消/Drop释放原snapshot charge；epoch失败、Source原Err和完整/不完整paired
的不同规则保持。默认manual fallback和USB发布核验同样接入共享额度。

publication分别持原64MiB子额度与共享额度，两份原permit均由table row和original
native registry共享同一Arc。只有表与已认证joins的native owner都释放才归还；
停止/clear、表先移除、native先join、未发布连接取消均不能提前归还。局部额度失败
不替换原Source错误，不认证设备ACK；被拒新native owner仍由原retirement收尾。

新增4项与原3项native charge合同加强：共享池同时收费USB/paired/publication、
预算拒绝不改变已有charge；USB跨paired wait/caller Drop释放；拒绝raw USB保留
旧publication且不新增paired/connect；真实userspace新owner拒绝、原joins、strict
peer EOF/task baseline、snapshot独立持有，以及两份原permit最后owner释放。
原metadata/native/epoch/deadline/source错误门禁集中重跑；具体通过/忽略数、严格
结果为369通过/2原ignored（Playwright browser、显式L04测量），包含新4项。
严格CLI Clippy/native CLI build、实际readonly-policy manager 1项、fmt/diff均通过；
主机runner 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
readonly-policy实际CLI不启动Source发现，trap零连接；不能据此认证capacity故障wire
或设备工作流。其它未变门禁按来源复用，不称全量Rust/SDK/平台重跑。
源码、受测二进制、完整日志和精确提交关联由外部summary登记。

首轮metadata过滤门禁35通过/1失败：新增fixture用缩小到单row成本的global池，
却先调用还需要USB快照余量的Usbmux启动，因此未到目标拒绝阶段。改用Wi-Fi路由
建立原publication后测试USB拒绝；原零余量、旧Arc与Source调用/最后释放断言保持。
失败日志保留，不通过提高fixture总额度掩盖需要验证的共享池边界。

## 精确剩余范围

128MiB是上述原raw snapshot与publication/retirement admission charges的总额度，
不是整个manager heap、进程RSS或全部metadata上限。Source返回前分配、selection/
candidate/HashMap/backing、ticket/intent、failure/retry/scan历史、原error payload及
原生runtime内部buffer仍需要自己的准确计量和统一归属；未计量dyn Error保持
1e14082的opaque/未join/active-scan标志，不计成0字节或用固定槽位数量代替。
本项不关闭L02，也不将完整refresh/reuse/restart/manual epoch或真机条件豁免。

账本保持23交付/6缺开发/L02待完整收口，有效30；Q04及全部子项排除。各可验收
项验证后立即本地提交；无子agent、push/发布/部署、设备发现或设备变更。
下一步推进error/history及其它已知metadata owner的字节归属/准入/最后释放；
任何完整范围仍缺代码都继续登记未完成。

外部证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12eff-9622-7f01-b777-335a6d20895d\repair\l02-global`。
入口`global-known-metadata-summary.json`、`round-handoff.json`，源码与失败日志保留。
