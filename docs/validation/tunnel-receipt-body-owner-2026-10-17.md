# L02 FailureReceipt body final owner · 2026-10-17

原 FailureReceipt 的 body/selector permit 位于 raw Arc value 内，不能证明
allocation 释放后才退款。本项统一接入 Receipt=shared_body::Strong<FailureReceipt>，
覆盖 Ledger、Attempt、SourceFailure、Removed cause、payload snapshot 和写入批次
的全部引用。数据 Arc 在前、body guard 在后；字段按最后 strong 的原语义结束，
Weak 观察引用继续保留 backing 的 body 费用，到最后 Weak 释放才退款。
[Weak 的 value/backing 合同](https://doc.rust-lang.org/std/sync/struct.Weak.html)
明确区分这两段寿命。当前 Weak API 仍仅用于 cfg(test) 观察，生产不暴露 raw Arc。

原整笔预留包含实际 receipt body、新增 guard 已知 payload、UID/host；先拆
body grant，Original/wrapper 的全部 capsule 费用仍先于任何 selector 复制和
Source poll。剩余字段 credit 使用原 Copier，完成后交给值内 selector permit；
独立 body grant 在创建数据/guard Arc 前校验同 bank/精确金额。没有新 bank、
重复 body 收费或短 sidecar。snapshot/WriteBatch/Removed 的 backing/helper 使用
实际 Receipt 宽度和 body 大小，不保留旧 Arc 指针尺寸假设。

原 typed IO、first/latest capsule、同 Ready poll 捕获、pending/current/retired
根、completion/epoch/deadline、失败表与 native 回收保持。Strong 提供原 Debug/
Display 转发及私有地址比较，原隐私与同对象断言保留；没有 raw Arc getter。
其它 freshness::Receipt 和 WebSocket cause 是不同 owner，不作混同修改。

新增 3 项专项通过：body/all-capsule 差一字节拒绝零 selector 分配、funded
零余量下原地址与最后 Weak 两阶段退款；实际 Ledger snapshot/report 在清理
后保留同一 typed IO 和 Pending 状态，最后 snapshot 结束后 body 仍等 Weak；
pending Source 跨 lifecycle clear/ledger close 仍捕获原 IO/Abandoned，retired
root 与8线程最后 report 释放后，body 在最后 Weak 才准确退款。

首轮编译发现 Display 转发和一处旧 Arc 地址比较遗漏，已补齐；次轮专项
2 PASS / 1 FAIL 是新 standalone Ledger 测试误期望 Recorded。原语义由后续
bookkeeping 写入认证 Recorded，单独 Ledger 保持 Pending；仅修正新测试预期，
生产状态不改。失败日志保留。最终专项 3 PASS，完整 tunnel 704 PASS / 0 FAIL /
2 原 ignored，upstream access 10 PASS；all-features/all-targets CLI Clippy
-D warnings、build/fmt/diff、实际只读 manager 1 项通过。原精确预算、隐私、
原 IO pointer、回滚及 native join/严格 EOF 断言未放宽。

Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
657 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`6ac991399d68f1096542f620cf2db9ea8792ce59afcae1486f3c7486e612ad5a`。
最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-receipt-body\round3`。

Arc 私有 header 精确字节、其他 route/框架 owner、anyhow/opaque IO 分配、发现/
SDK 原输出、公开初始 RsdHandshake 和完整 freshness/刷新/复用/重启/manual epoch
审计继续。128MiB 为已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/
6 缺开发/L02 待全审计、有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
