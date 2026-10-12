# L02 candidate输入/backing与最后owner · 2026-10-12

完成验证续记：2026-10-13（Asia/Shanghai），沿用本项开始日期的证据文件名。

承接 `9e0eae7dcc2efefa1ff0bf97b4bac91abc5edeab`，原checkout/分支保留，起始
工作树干净；536个构建来源文件及CLI SHA256重新核对一致。默认unit Source保持
NoDevices，专用localhost fixtures继续原Source/native IO与joins/严格EOF断言。

## 生产交付

candidate window使用KnownVec按实际capacity收取同一128MiB bank；增长时新旧
backing同时收费，拒绝保留原Vec/row，into-iterator持charge直到原backing释放。
原4096 count/逻辑text预算继续，不能用这些计数替代字节准入。

输入有一个共享Arc owner，按原UDID/Route String.capacity与inline Input收取额度。
Source消费的一个route clone及延期intent的一个UDID clone在Source开始前预留；
Source route clone实际capacity须在已预留范围内。USB原String直接移动，原raw
snapshot permit拆分其capacity并合入Input charge，不给同一原buffer再收费。

paired先完成整批逻辑与实际输入/backing准入，再创建automatic tickets或并入
已有USB window。拒绝整批为Incomplete，丢弃其新输入与raw charge，保留USB候选
及未知paired状态，不留准入前缀。当前域有权限的manual/automatic规则和epoch
fence继续，旧Source错误仍独立保留。

table publication、原native registry（从reservation开始）、延期intent与原Source
错误context共用同一Input owner。table移除不能提前释放未join runtime的Input；
原joins先完成也不能释放仍由table持有的Input。Source Err加入仅含安全固定文案的
context，保持原错误/source/downcast对象；receipt或caller保留该context时，manager
clear/Drop仍不退款。最后owner释放才归还，未用Drop、RST或计数认证设备ACK。

原错误payload census额外遍历这一真正被保留的Input buffer并按原地址去重，不
改变其采样/opaque/未join边界；这是独立的观测，不能替代任何准入判断。

## 验证与剩余

新Input 4项已通过（容量/原buffer、共享最后释放、USB charge转移、精确拒绝/错误
bank、原IO下转型/context owner）；新增production Source receipt最后owner和2项
原userspace native table/joins释放顺序也通过，严格peer EOF/task baseline保持。
KnownVec增长/计数/iterator原backing门禁以及受影响完整tunnel、严格Clippy/build/
实际readonly manager/host runner的最终结果由外部summary登记，未跑不写PASS。

修正后受影响tunnel397通过、0失败、2原ignored；新增9项（4 Input、2 KnownVec、
2原native释放顺序、1原Source receipt最后owner）均实际通过。严格CLI Clippy已通过；
两个ignored仍为browser及显式L04测量，不称全部Rust/SDK重跑。

严格CLI build、fmt/diff及实际readonly-policy manager1项通过，独立localhost trap
零Source发现连接；host runner为5 PASS、0 FAIL、22 NOT_RUN，
real_device_executed=false。原stdout/stderr/exit与raw/独立masked扫描合同沿用已验
capture源码；本项不重跑未变SDK/core协议和其它平台。

首次完整tunnel为396通过/1失败/2原ignored：批次预准备把原每行权限重查前移，
导致在首个ticket等待时被撤销授权的B仍建连（实际A/B/C调用为1/1/0，原要求1/0/0）。
恢复每行ticket前后权限重查，仅移除已经准备的输入，不能增加新授权但未规划的C；
原测试和精确调用断言不改，失败日志保留，修正后重新执行完整受影响门禁。

原门禁的精确shared-bank fixture加入本项新增的真实charge，继续保留零余量、
一字节不足、原backoff version/slot/identity、源调用计数、原对象和收尾断言。
payload census原CoreError 4096/8192容量不变，额外计入实有Input的UID buffer，
不删减原容量断言或将不透明payload改写成零字节。

仍缺原error payload的完整创建来源/字节准入；Source产生的独立错误buffer、
anyhow/Arc/watch内部、failure/retry/scan/native history及其它集合backing、outcome
collectors与manual desired/route副本继续。Input的Source/outcome clone预留可晚于
实际buffer释放而保持到最后Input owner；它是保守所有权reservation，不是RSS。
所有单项上限不能保证同时填满128MiB pool，额度竞争仍按typed拒绝处理。
完整freshness/刷新/复用/重启/manual epoch继续，不能据本项关闭L02。

账本23开发交付、6缺开发、L02待完整收口；有效30，Q04及全部子项排除。
只本地提交，无子agent、并行Cargo、设备操作、push/发布/部署。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-candidate-owners`。
