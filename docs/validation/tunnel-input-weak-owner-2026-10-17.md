# L02 Input strong/weak body owner · 2026-10-17

Input 原先将 body/selector 费用放在 Arc value 内，最后 strong 退款时 Weak
仍可保留原 allocation。本项将全部私有 Input 引用改为封闭 Owner：数据 Arc
在前、独立 body guard 在后，所有 clone 一起持有两者。字段仍在最后 strong
结束时正常析构；Weak 只保留 backing 的 body 费用，upgrade 返回 None，不保活
Input、原 key 或 native 资源。最后数据引用释放 allocation 后，才结束对应
guard 引用。该次序依据 [Weak 的 value/backing 寿命合同](https://doc.rust-lang.org/std/sync/struct.Weak.html)。

guard 使用已验证的 strong-only Shared<OwnedSemaphorePermit>，自身 Arc allocation
释放后才析构原 permit；其新增已知 payload 大小也在创建前计入 body_bytes，
没有未付 sidecar。Input 原 body grant 从原预留拆入 guard，selector/reserve
费用仍在 Input 字段后，实际 outcome key/grant 独立移动。构造先校验原 bank
及精确 body+guard 字节，再创建两处 Arc；字段局部绑定在费用后，拒绝/部分
失败中字段先析构。原 USB 字符串指针、raw grant split 和 key 目的地保持。

全部引用路径已接入，包括 candidate/seen、Observed、Node/success、连接 context、
ownership/native registry 和 SourceInputContext。backing/header 预算按实际
Owner 宽度，body helper 包含实际新增 guard payload；没有旧 Arc 指针宽度
假设。原错误 pointer、epoch/deadline、Source 前准入、失败历史、partial USB
和 native joins/严格 EOF 规则保持。Weak/downgrade/upgrade 当前仅用于 cfg(test)
观察 seam；生产路径持同一 Strong/guard，不暴露 raw Arc、Weak 或未持费入口。

新增 6 项专项通过：高对齐128的原值/字段指针，最后 strong 单次字段析构、
最后 Weak body 退款；8线程 Weak upgrade 与最后 strong 竞争；字段析构 unwind
后 body 仍持费到最后 Weak；异池/短/长 body grant 拒绝及单次析构/退款；ZST
仍计 guard 已知 payload；实际 Input 的字段/outcome 先结束、body 等最后 Weak。
既有弱引用观察测试已加强为两阶段精确断言：upgrade 失败后仍扣 body_bytes，
显式销毁最后 Weak 后全额恢复。原错误、key、native 句柄/任务和 EOF 断言
未移除或放宽；原结构预算仅改用实际 Owner/body 类型尺寸。

最终专项 6 PASS，完整 tunnel 699 PASS / 0 FAIL / 2 原 ignored，upstream
access 10 PASS；all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、
实际只读 manager 1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。655 个冻结源码无漂移；core/Cargo 未改，
既有 core 证据复用，不声称重跑。受测 CLI SHA-256：
`059fd8d397d2171ba4139039cef449c2292769e8fc96a28d6afffc77e27dbc54`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-input-weak\round1`。

本项没有计入私有 Arc header/controlblock 精确大小，其他 receipt/SDK Arc 不
由此自动证明。其余 Arc body/weak、发现/SDK 输出、初始公开 RsdHandshake、
HTTP/client/config/framework/private IO 和完整 freshness/刷新/复用/重启/manual
epoch 审计继续。128MiB 为已准入对象池，不代表完整 heap/RSS；L02 未关闭，
23 交付/6 缺开发/L02 待全审计、有效 30 和 Q04 全部排除保持。仅本地提交，
无真实设备、push 或发布。
