# L02 USB attachment/selected 索引字节归属 · 2026-10-13

生产 scan 的 raw attached HashSet 和 selected attached_ids HashMap 只借用
UID，却仍有未收费 backing。本次统一为 usb_index::Index：借用原 UsbDevice
slice，实际 Vec<usize> capacity 在已有共享池准入后才填充/排序。无UID复制。
raw membership 先binary search相同UID范围，再匹配numeric device_id；每个
publication UID唯一，整个retain中的范围互不相交，不做rawRows × publications
的全扫描。selected lookup保持旧map同key最后输入条目的规则，private重复
输入也不按numeric ID大小选错；原selected_devices唯一与firstUSB/last规则保持。

raw index在原表retain后释放，随后才将原snapshot交给selection；selected index
覆盖manual route解析后释放，随后才移动selected原UID到自动candidate输入。
两个索引都借用原已准入buffer，保留相应raw/selection owner，真实backing费用
只到索引最后owner释放，拒绝不改变原buffer。索引构造不在全局table锁内分配
或排序，原借用lookup在短table同步commit中执行，无网络等待。

新增两项测试检查真实费用、exact raw pair、同key最后输入规则、原UID buffer、
一字节不足完整拒绝/退款和empty index零存储。Root完整串行tunnel门禁442
PASS / 0 FAIL / 2原ignored。原Source错误、epoch、deadline、backoff及native/task
joins与严格EOF断言保持，旧quota夹具和断言均未修改。严格CLI Clippy、CLI build、
fmt/diff通过；实际只读manager1通过且Source连接为0；主机runner5 PASS /
22 NOT_RUN / 0 FAIL，real_device_executed=false。未变Core/F04/SDK及其它平台
门禁按影响复用或保持NOT_RUN，不称全部范围重跑。

这只覆盖上述两种索引的真实自有存储。seen、pending/planned、paired observed、
额外failure table key副本及owner/rollback、private error/native/runtime和完整
freshness/reuse/restart/manual epoch仍缺，不关闭L02。原candidate Input已为
一个outcome UID clone预留费用，不重复宣称该费用未交付，也不能推导其它副本
都已计量。账本23交付/6缺开发/L02待收口，有效30，Q04全部排除。

外部原始日志、555个build-source冻结与后续提交proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-usb-attachment-index`。
