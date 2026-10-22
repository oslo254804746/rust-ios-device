# L02 CLI private watch complete roots · 2026-10-22

CLI 原 global/scan/device/desired intent/bool shutdown watch 使用不透明 Tokio
共享分配，缺少完整根计费。本项将这五类实际私有通知命名空间改为
Strong<State<T>, ()> 声明根，复用既有完整 strong/implicit-weak 原位析构、
最后实际 dealloc 后退款的封装；原 bank 准入 value RwLock、独立 version/
closed 原子、sender count、inline Notify 和实际根控制/费用字段。未新增 unsafe
或估算标准/Tokio Arc 私有头，也未更换 bank、增大128MiB/4096固定限额。

所有原 Arc generation 值及其指针比较、clear/stop/reset 的发送点、原
authority/epoch/intent 锁序与 biased select/期限/Source 原 IO/真实 joins
保持。私有 Closed 仍仅被忽略或映射到原 OperationCancelled；不声称其为
原生 RecvError。Core TunnelHandle 等公开 watch 继续 Tokio，包括 retirement
的 Receiver<()> fixture。本项只实现实际使用的私有 API，未声称完整 Tokio
watch 公共 API 兼容。generation Arc 值本身的分配仍属原待办范围。

Root 使用 std RwLock<T> 读 Ref 作为原同步提交 fence；version/closed 独立
AtomicUsize。GlobalEpoch.commit 持 Ref 后查询 has_changed 不再次锁 value，
也不引入 meta→value 的 ABBA 次序。borrow 先读锁再看版本、不消费 seen；
borrow_and_update 仅更新本 receiver。subscribe 视当前版本已 seen，clone
保留旧 seen；每次 equal/no-reader send wrapping +2，最后 sender 仅设
closed，不推进版本。changed 版本优先于关闭，Notified 创建早于版本检查，
解锁后 wake-all，取消不消费 seen，并保留 cooperative wrapper。

send_if_modified false 保留原地修改但不推进/唤醒；callback panic 先正常
释放写 guard 再 resume 同一原 panic，不虚构回滚/poison/通知。send_replace
返回 exact 原 T，在 guard 外析构。manager 原 wait_for(|bool| bool) 保持
单个 cooperative wrapper，检查已 seen 的当前值/最后关闭值，谓词前标记
seen，false/panic/cancel 的行为依 pinned Tokio 1.50.0 差分验证。单 Notify
取代原分片结构，不声称相同性能或已经计入 opaque waiter/Waker/任务内存。

三个初始根（global/scan/bool）与每 gate 两根（device/intent）独立计费。
Lifecycle 两根先整笔准入再首次 backing；新 gate 的原 body 与两 watch 根
在 key/table 前整笔预留，原 native permits 精确拆分后进入各原 backing；
不足时保留原 key/backing 拒绝阶段，未晚补费用。私有 Result 构造沿现有
manager/ticket 的首次 Source 前路径传播。配置测试重建 bool 根也使用新
原 bank，先证明 unique/initial/false/无 observer，原新 grant 在替换前存在。

新增11项验证（watch10、真实 lifecycle constructor1）覆盖原 Arc 返回值/
borrow/seen/clone/late subscribe/zero receivers、关闭最后 unseen、partial
false mutation/typed 原 panic/no poison、wait_for 当前/关闭/取消/谓词 panic、
持 Ref 阻写与独立 metadata 查询、注册后取消、预算耗尽、invalid/N-1 grant、
exact root/address/闭池 clone、原 !Unpin value 原位 Drop/原 panic、最后
Weak 实际释放后退款、最后 pending receiver 取消和回绕。实际 lifecycle
整笔 N-1 观察零 backing，exact 两根首次分配前全额扣费，旧 global/scan
views 在 lifecycle 结束后继续持有原费用，最后 view 结束再退款。

只读 agent 协议/实现审查未发现生产缺陷，但指出主动重新 poll 不能证明
wake-all。主控据此保留16个独立持久 Wakes 计数，在任何重新 poll 前逐个
核对 send 和 last-sender-close 确实唤醒全部，再比较原 Tokio 结果；此真实
测试空档已修复。审查者未编辑源码、未运行任何 Cargo。源码审计报告及精确
primary SHA 在外部 cli-watch-root-audit 和 cli-watch-implementation-review
中；运行证据另列，未以静态结论代替测试。

旧精确 fixture 按实际根尺寸及真实寿命更新：gate trace 从3到5个实际
backing（key/table/device watch/intent watch/原 DeviceRow），原字段地址
与所有费用均核对；table/scan 可先退款而旧 ticket 持有 global/gate 根；
intent receiver 结束前 watch 费仍扣；route snapshot 不持有已结束的通知根。
零额度 stop/reset fixture 先支付实际通知根，余额归零且闭池后执行原控制，
观察零新受控 backing，最后同一根精确退款。固定额度没有变化。

R1 首个 strict 测试编译发现4处 std Cell 误适配与新测试临时 Ref 活得过长，
仅修正测试。R2 strict/watch10/weak4通过，lifecycle21通过/6预算失败；主控
补整笔根前缀预留及精确 fixture。R3 lifecycle28通过，intent3通过/7预算失败，
仅补实际 root/receiver/snapshot 寿命；R4各定向门禁通过，完整CLI1779通过/
2 manager minimum 旧公式失败，仅补实际三个初始根。R1–R4日志保留，无
删除原拒绝/指针/退款断言或扩大固定池；生产逻辑自R3至R5保持。

R5 全部13个命令通过：CLI all-features/all-targets Clippy -D warnings，
private watch 10、原 Weak root 4、
lifecycle 过滤 28、intent 10、global metadata
17、manager owner 4、完整CLI
1781 PASS / 0 FAIL / 13 原 ignored、build/fmt/diff、实际只读 manager1
（Source/usbmux trap0）、host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。全部 Cargo/build 由主控串行执行。

692 冻结源码无漂移，18处差异仅CLI；Core/Cargo.lock哈希与160cf59一致，
Core沿用846c533已验tunnel159/owner42/无mDNS37/watch10/无mDNS10及29门禁，
未声称本轮重跑Core。实际 host 与归档二进制 SHA-256：
b77b77901ba537f430910b6d1be4c0b505797c94286d1dc15e62935d7425b662
source delta、verified-ios.exe、current/提交后 rawGit proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-cli-watch-roots\round5

Arc generation marker、Core公开 native Arc/Weak/watch、opaque error/report/
wrapper、SDK/发现/metadata副本、HTTP/WS/配置/凭据、框架/allocator/waiter/
Waker及完整各模式endpoint恢复继续原范围。128MiB为已准入已知对象池，非
整个heap/RSS。未声称Miri、其它平台或真实设备验证。L02仍待全审计；主项
23交付/6缺开发/L02待全审计、有效30及Q04全部排除不变。仅本地提交。
