# L02 USB selection backing准入 · 2026-10-12

承接 `0ddd8c1a67f1c44ad9e445697edb3a9020cf7494` 的coordinator字节归属，本项
将生产扫描的USB selection临时HashMap和key克隆改为已准入的索引Vec。

## 生产规则

先按原raw snapshot完整准入，再为索引Vec实际capacity和结果Vec实际capacity
分别取得同一128MiB bank的charge。选择期间原raw Vec、索引、输出backing同时
占额度；索引在选择结束释放，输出reservation由scan持有至该次扫描退出。
原raw charge继续覆盖从selection移动到candidate的原String容量，未复制key。
这是一项保守准入reservation，输出Vec在candidate交接后可能先释放，而其原charge
保持至scan结束；不声称实时allocator/RSS字节等于占用permit。

索引按借用的UDID和原位置排序，组内首USB优先、无USB选最后网络记录，再按每组
原首次出现位置恢复调度顺序。输入的原String buffer直接移动，不丢失spare capacity。
准入是同步try-acquire，不持全局锁进行网络等待，不新建worker或后台重试任务。
拒绝时返回typed ResourceLimit并释放本项已获取的scratch/output charge；原raw、
publication、epoch、原Source errors和native retirement规则继续。

## 门禁与准确范围

新增5项门禁实际通过：首identity顺序/首USB/最后网络、原buffer指针和capacity、
精确并发峰值/一字节拒绝/refund、空输入spare capacity、128种有重复的输入序列
与原选择规则独立比较。已有shared-bank scan fixture保留零余量、caller Drop退款、
paired原task退出和Source调用断言，加入本项的准确reservation。完整受影响tunnel
378通过、0失败、2原ignored。严格Clippy/build、fmt/diff、实际只读manager1项均通过，
readonly策略跳过发现且localhost trap断言零连接；host runner为5 PASS、0 FAIL、
22 NOT_RUN，real_device_executed=false。原browser和显式L04 ignored保留，不称全部
Rust/SDK重跑。精确source/binary/log/commit由本项外部summary登记。

仍未交付：其它candidate/seen/attached/manual/initial/failure/retry/scan/native backing、
manual route独立克隆与原error payload的完整创建来源合同/字节准入。与前项一样，
allocator峰值、Arc/watch内部heap、Source返回前分配和真实device状态不由本项认证。
L02完整freshness/刷新/复用/重启/manual epoch仍待核查，不能由本项关闭主项。

账本23开发交付、6缺开发、L02待完整收口；有效30，Q04及全部子项排除。
Rust/Cargo串行；本项仅隔离localhost fixture，无子agent、设备操作、push/发布。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-selection`。
