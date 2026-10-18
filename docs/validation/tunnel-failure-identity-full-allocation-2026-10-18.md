# L02 failure route identity full allocation · 2026-10-18

failure table 原 IdentityOwner 保留 AttemptIdentity 与组合费用，但共享对象仍由
std Arc 分配，控制块未计量。本项改用应用声明的精确共享分配：counter、原值、
原 grant 与 padding 的真实 layout，加上受控 route 字符串的实际字节容量，全部
在复制/创建前从同一原 bank 准入。不估算 std Arc 私有布局，不增加费用 sidecar。

shared_allocation 新增 distinct from_grant_with_fields：验证同池与完整
actual allocation layout + known fields 后，暂拆精确 body grant，调用原 exact
allocator，再把原 field 部分合回同一份 grant，随原 root 保留。验证拒绝路径先
销毁传入字段，再退其费用；最后一个正常 owner 释放完整 body、销毁原字段后
退款。原 zero-field from_grant 仍严格要求 exact body layout，没有放宽成任意
extra quota。实际 allocator 与引用计数/Release-Acquire/Unpin/noWeak/noRaw/
noMut/noPin 规则保持，没有新增 unsafe。所用 tokio merge 的同池与加法条件均
在分配前校验，原池关闭后也只消费已取得的 grant，不创建新池或申请新额度。

IdentityOwner 仅保留原 AttemptIdentity。生产 copied 路径先付 whole root + UTF8
route 长度，再用原 Copier 创建 exact capacity 缓冲区；Clone 分享该 root，
不克隆 String。Node 的实际预算计入完整 root。测试适配器 owned 路径仅移动
已存在的原缓冲区，按实际 capacity 留费；不据此声称外部输入的创建已准入。
原 PartialEq/Borrow、native_id、原 key/route 指针及 failure row rollback 保持。

新增3项通过：完整字段/控制块使用一份原 grant，在每次 backing 前付费，关闭
原池后的已准入构造与最后 owner 原 buffer/费用寿命；异池、错误字段尺寸、加法
溢出拒绝不分配，输入先销毁再退款；真实 identity 少1字节在 root/字段复制前拒绝，
移动4096容量原缓冲区保持 pointer/capacity/native_id 并在原池关闭后准确退款。
原8线程 identity 原 host 指针和一次退款保留；原严格零字段/256对齐/并发clone/
原析构展开测试保留。原 Node、selector 与 outcome key 分配观察器现在同时检查
新控制块，仍精确核对原 Node/key/string 地址、两项真实布局、重复拒绝后原 Node
不变与 byte bank 恢复，没有忽略新增分配或允许 key 的额外副本。

共享分配6、failure table13、outcome key4、reconnect21、error payload9及严格
CLI all-features/all-targets Clippy -D warnings 通过。完整CLI1740PASS/0FAIL/
13原ignored，build/fmt/diff与实际只读manager1通过；Source/usbmux trap0。
host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。Core/Cargo未改，既有
Core证据复用，不声称重跑。

首轮定向门禁通过，完整CLI1739PASS/1FAIL/13原ignored：旧 outcome key 观察器
只允许Node body，误将新完整identity root识别成key副本。现对成功/重复拒绝两条
路径都断言恰好两处实际分配、原Node地址和identity值落在对应root范围内，并保持
原目的地址、内容与配额退款；生产代码未因此改变。第二轮全部12个串行门禁通过，
681个冻结源码无漂移，5处源码差异。CLI SHA-256：
`f467998485649e5fa69e1cbd64b9f9c57265f0e26816a015a4ed50ba4e038122`。
日志、source delta、受测 binary、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-failure-identity-full-allocation\round2`。

本项仅关闭 failure route identity 的完整共享分配及其原字段费用。Ledger、其它
owner/marker/private headers、原 opaque错误/Anyhow/context、SDK/发现输出、
框架IO/task/allocator与完整freshness继续原范围审计。128MiB是已准入已知对象池，
非完整manager heap/RSS；L02未关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部
排除保持。仅本地提交，无真实设备、push或发布。
