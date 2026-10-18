# L02 publication two-budget body · 2026-10-18

Publication Life 有真实 Weak 持有路径，原两池 permit 位于 raw Arc payload，
只能延续到字段析构。本项将全部 table/local target/scan snapshot 根接入
paired LifeOwner，原 data 引用先于独立共享 guard 结束。两池均在创建前
预付实际 sizeof<Life> 加 guard 两个 permit 的 payload，最后 strong 及时
结束字段，最后 Weak 结束原 data allocation 后才释放 guard 并退两池费用。
guard 自身 allocation 由既有封闭 Shared 在 permit 析构前结束；没有免费
sidecar、private Arc header 估计或额外任务。

原 closed flag、Notify、同 poll Ready 原错误、工作取消及双池 Admission
类别保持。creation hook 仍观察原 Life 地址及创建前费用，snapshot 计数
仍为原 data strong 数量；所有原地址比较保留为封闭 owner 的 ptr_eq，
没有改动 catalogue/native handle/typed IO 的 Arc 比较。实际 row/table
结构按新类型宽度收费，原 Source 前拒绝和零余量规则保持，无额度 padding。

新增2项通过：关闭原 pending work 时原 future 及时析构，闭池/零余量下
两笔 body 费用仍由 Weak 保留至最后释放；8线程 Weak 升级保留原地址和关闭
状态，最后原根后 Weak 不能升级，两池最终准确退款。原 native 测试加强为
table 移除后两池 body 仍持有，显式 Weak Drop 后退款，再继续原 input/route
阶段与实际 joins/strict EOF/task baseline 检查，不把 stop 当作 joins。

Publication全组7、scan snapshots3、native metadata6、完整 tunnel733 PASS /
0 FAIL / 2原ignored、upstream access10通过。all-features/all-targets CLI
Clippy -D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux
trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
round1编译失败记录保留：native intent 的别名 published 仍使用 Arc::ptr_eq，
已改为同一 data 地址比较，原 failed_empty/身份断言保持；round2全部重新验证。

667个冻结源码与当前worktree一致；core/Cargo未改，既有core证据复用，不声称
重跑。CLI SHA-256：
`4e57679533b0b5f19abf6358d9b018912bb9f7ba31a0aac88bb2d6c83cf39e58`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-publication-dual-body\round2`。

Lifecycle/其他 owner、terminal/anyhow/private header/opaque error/IO、发现/
SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为
已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与
Q04全部排除保持。仅本地提交，无真实设备、push或发布。
