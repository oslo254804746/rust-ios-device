# L02 freshness Receipt final owners · 2026-10-18

Freshness Receipt 原 body grant 位于 raw Arc payload 内。完整根审计覆盖
Source 局部、两份 Failure report、registry first/latest/orphan、swap 输出
及 census roots，未发现该根的 Weak/pin/raw Arc 出口。本项统一为封闭
ReceiptOwner，最后原 allocation 结束后才析构 Receipt/原 IO/body permit。
report 使用稳定的 owner 专用实现，仍共享同一原根，没有复制原错误。

原工厂在 Source 前一次准入实际 sizeof<Receipt> 加两份 sizeof<Failure>，
原两次 report 上限、费用拆分、同 Ready poll capture/retain 顺序保持。
Root/Census/Slot 按实际新类型尺寸收费，没有新 guard 或额度 padding。
原 Source typed IO、source chain、错误类别、绝对期限、epoch、原 lease、
刷新回滚、取消与 native joins 保持。外层 anyhow Failure allocation 的
物理释放/私有 header 是独立未完成边界，不据本次根 owner 修复推导完成。

新增2项通过：零/整笔差一字节拒绝及精确 Source 前 body/两份 report 准入，
关闭银行/零余量下两份报告保留同一 root、原 IO 指针与类别，独立快照在
报告结束后继续保留原 IO，原 IO 析构时 body grant 仍持有，最后全额退款；
8线程最后 root 并发释放保留同地址/原 IO 和未使用 report stock，单次析构
并准确退还原池。没有替换原 completion 状态或放宽原 Native 断言。

Receipt专项2、native freshness全组14、完整 tunnel731 PASS / 0 FAIL /
2原ignored、upstream access10通过。all-features/all-targets CLI Clippy
-D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux trap0，
host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

667个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`8c1f6424c11e52c22e5420c9146e54bf75270020d42dd83a42964550f4d6efb1`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-fresh-receipt-owner\round1`。

Publication 等 owner、terminal/anyhow/private header/opaque error/IO、发现/
SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为
已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与
Q04全部排除保持。仅本地提交，无真实设备、push或发布。
