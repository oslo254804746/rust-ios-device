# L02 reconnect Removed body final owner · 2026-10-17

原 reconnect Removed 将 body 和 selector 费用合在 Box 内，不能证明 body
allocation 已释放才退款。本项改为 OwnedRemoved=owned_box::Owner<Removed>，
覆盖 episode、receipt、State rows/causes、linked next、RemovedBatch、displaced
和返回值。所有 backing/initial-attempt header 预算按实际 wrapper 宽度计算，
不保留旧 Box 指针大小假设，数量、顺序和调度规则保持。

episode 仍先取得原 body+UID+host 整笔费用，从中拆出精确 body grant；新增
Destination::from_grant 使用同 bank/精确 Layout 的原 grant 创建实际空 Box
目的地，不重复收费。其余字段 credit 交给 Copier，完成 UID/identity 复制后，
值内 permit 只持 selector 费用，body permit 位于原 Box 外。receipt 工厂同样
使用原 body grant，其值内 permit 为同池零字节余量，不另请求额度；原 cause
Arc 直接交接，没有复制 payload 或替换原错误。

原空 body 指针在 place、表和回滚中保持；Box 析构及原 allocation 释放后才
归还外部 body grant。实际 selector 字符串和 cause 仍先于各自字段费用/owner
结束。需要移出值时复用已验证的 into_inner，body 可结束而原字段继续持费。
全部 capsule/节点仍在 Source poll 前准入；原 pending cause、typed IO、版本、
deadline/epoch/native lease/joins 和严格 EOF 保持。

新增 5 项专项通过：原 grant 在零余量下创建精确 Box，同一指针，异池/短/长
grant 分配前拒绝；episode 移出后只退 body、原 selector 指针和费用保留；cause
body 差一字节拒绝零分配及原 receipt Arc 交接；后续 row constructor unwind
时实际空目的地/字段准确释放；实际 ledger 在零配额下恢复原 row/cause Box
地址，最后批次 owner 退 body/selector，原 IO receipt 仍保留至最后报告。
旧 selector hook 现在核对精确 body Layout/原指针再核对 UID/host，不隐藏新增
实际分配；原精确预算和 native 断言未放宽。

首轮专项 4 PASS / 1 FAIL 是新 fixture 使用默认 legacy 策略，完成后不保留
episode row，批次首项为 cause 空 key。该测试改为显式 max_attempts=1 的有界
episode 策略以核查持久 row 回滚；生产默认行为和原默认策略测试不改。首轮
日志保留。最终专项 5 PASS，完整 tunnel 688 PASS / 0 FAIL / 2 原 ignored，
upstream access 10 PASS；all-features/all-targets CLI Clippy -D warnings、
build/fmt/diff、实际只读 manager 1 项通过。Source/usbmux trap 0，host
5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

651 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`217ec9885287cca4b2742034b809595568030ce3455fc6757addb0c29118cdd5`。
最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-removed-body\round2`。

共享 Arc payload/weak/controlblock 的最后 allocation、发现/SDK 输出、公开
初始 RsdHandshake、HTTP/client/config/framework/private IO 与完整 freshness/
刷新/复用/重启/manual epoch 审计继续。128MiB 为已准入对象池，不代表完整
heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30 和 Q04 全部
排除保持。仅本地提交，无真实设备、push 或发布。
