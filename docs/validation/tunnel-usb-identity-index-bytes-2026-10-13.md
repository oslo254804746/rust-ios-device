# L02 USB 原快照唯一身份索引预算 · 2026-10-13

USB 快照校验原先用未收费 HashSet 借用原 UID，检查4096个唯一身份上限。
本次改成实际 Vec<usize> backing 的排序索引：先验证原字段/容量和总存储，
再取得同池 raw reservation，临时索引真实 capacity × usize size 成功准入后
才填充、排序和计数。全 raw 输入继续验证，不根据授权/选择的子集放宽上限。
原USB发现顺序、UID buffer、first USB/otherwise last的选择规则均保持。

临时索引的原permit覆盖sort/计数和iterator，成功后只返回raw permit，索引
真实存储先释放、随后其费用释放；任何 quota/唯一身份拒绝都释放raw与index
owner。无独立UID副本或opaque backing。两处生产consumer共用同一准入函数；
无生产consumer调用不带共享池的逻辑validator。仅原逻辑validator单测保留
cfgtest入口，不由它认证生产准入。原返回前Source分配与allocator rounding
仍不在本known-storage子项内。

Root 串行tunnel整组440 PASS / 0 FAIL / 2原ignored，包含新增2项：raw+index
总额度一字节不足，原buffer和顺序保持/费用全部退款，exact success只留下raw
费用；16384行重复身份接受、4097个唯一身份拒绝且原顺序保持。原snapshot
capacity、native owner、原error、严格EOF/task join、取消/期限及旧tinyquota
夹具/断言均保留。严格CLI Clippy、CLI build、fmt/diff通过；实际只读manager1
通过且Source连接为0；主机runner5 PASS / 22 NOT_RUN / 0 FAIL，
real_device_executed=false。未变Core/F04/SDK/其它平台范围按影响复用或保持NOT_RUN。

L02继续缺其余scan容器/key副本、原error/private Arc/allocator/native/runtime
创建合同和完整freshness/reuse/restart/manual epoch，不是整个heap/RSS上限。
账本23交付/6缺开发/L02待收口，有效30，Q04全部子项排除；无真实设备发现或操作。

外部原始日志、冻结与提交proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-usb-identity-index`。
