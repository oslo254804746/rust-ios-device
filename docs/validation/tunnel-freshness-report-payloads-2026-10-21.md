# L02 freshness actual report payloads · 2026-10-21

原Receipt为两份Failure的sizeof预留费用，但费用随Failure字段析构，不能证明
opaque anyhow外壳已释放。本项保留原Failure typed wrapper、source()/original()
与原Receipt根，独立ReportPayload实际Box在首次Source前真实预付并分配。
费用在该Box外，其完整dealloc之后才退款；外壳与其中Owner字段的完整layout
仍是原范围未完成项，不据此称整个Anyhow报告已完整计费。

Receipt::reservation_bytes计算实际声明Receipt根加两个ReportPayload布局。
原bank整份grant先校验精确bank/尺寸并分割，任一foreign/短/长在任何backing前
拒绝；随后两个实际空目的地和Receipt根共三处backing才产生。空目的地不持
Receipt引用，不形成cycle。Source ReadyErr先保存原错误，report仅取一个目的地，
释放mutex guard、克隆原Receipt并原位place，没有新的受控payload backing或
字节准入，闭池零额度也保持原错误。最多两份报告契约保持，第三次明确拒绝。

所有原IO及内部payload地址、class、source委托和Root身份保持；typed byvalue
Failure提取前后ReportPayload地址不变。一个报告结束时只退自身原费用；最后
报告若同时是Receipt最后owner，原错误先结束，未用报告目的地及Receipt实际根
按原顺序释放，最后已填报告Box结束后才退该报告费用。没有永久强引用环，
也没有用新的Arc费用guard延后另一报告的独立退款。

保留原2项Receipt测试并强化三份实际layout/原地址、首个分配前whole debit、
原bank/root fee、Source后report零backing、闭池/byvalue、两报告after-free信用
点和8线程最后Root释放。新增2项验证foreign/短/长whole grant零backing拒绝，
以及仅一报告的最后owner同时清理未使用目的地和Root，无cycle并精确退款。
native freshness只将两处旧sizeof预算公式改为实际reservation helper，保留原
两个独立IO失败→成功清理、跨期限/clear/撤权、metadata peak、原native joins
与严格EOF等全部断言，没有扩大固定额度或忽略/删除测试。

子Agent在外部生成proposal后额度中断；主控独立读审全部三文件diff、原共享根
Drop与报告生命周期，核对全部base/proposed/owned_box依赖SHA后应用、格式化、
冻结，并串行执行全部Cargo与cargo build。11个验收命令通过：Receipt4、native
freshness17、scan19、error census9、strict CLI all-features/all-targets Clippy
-D warnings、完整CLI1765 PASS / 0 FAIL / 13原ignored、build/fmt/diff、实际只读
manager1（Source/usbmux trap0）、host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。Core/Cargo未改，复用55f21fe的prepared Core8证据，
不声称本轮重跑Core/Miri/其它平台/真实设备。684冻结源码无漂移，3处源码差异，
实际host报告与归档受测CLI SHA-256一致：`437522428c0e24982d4babda3755c37a7cde4e025ff4f66073a41b216e26a19f`。
日志、source delta、verified-ios.exe、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-freshness-report-payloads\round1`。

原opaque Anyhow/Error/wrapper、其它marker/Core private headers、SDK/发现、
HTTP/WS/配置/凭据、框架task/allocator与完整endpoint恢复继续原范围。128MiB
仍为已准入已知对象池，非完整heap/RSS。L02未关闭；23交付/6缺开发/L02待全
审计、有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
