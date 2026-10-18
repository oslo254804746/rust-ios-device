# L02 manager initialization owner · 2026-10-18

TunnelAgentInner 原 Arc payload 未持有初始化 body grant。本项在原 global
metadata 池预付实际 sizeof<TunnelAgentInner> 加独立 guard 的 permit payload，
然后才创建原子对象和根 allocation。生产工厂保持 Result，拒绝传播原
ResourceLimit，无新池兜底、额外任务或 private Arc header 布局估计。

完整根引用图含测试 Weak 回调，统一采用 paired Strong/Weak owner；data
引用先于 body guard 结束。最后 strong 及时销毁 Source 等原字段，Weak
仍保持原 allocation 的 body 费用；最后 Weak 结束后原 guard allocation
先释放再退款。所有旧 Weak 回调、同 poll epoch/IO 和 get_mut 配置点已接入
同一封闭路径，无 raw Arc 出口。测试重配要求唯一 root，先取得新池实际
grant、安装新 guard，再结束旧 guard 并退旧池，原对象地址保持。

新增 4 项 manager 测试覆盖零/body 差一字节及子对象差一字节拒绝、精确
初始化且 Source0；原根/Source 字段与最后 Weak 的两个释放阶段；8线程
Weak upgrade 保持同地址/原池并单次退款；精确重配保留地址和新旧池退款。
新增 rebind 测试覆盖额度不足、strong/Weak 已共享拒绝与原 grant 保留。
精确 native/scan fixture 增加实际根初始化费用后，原可用字节、Source0、
零余量、原 native joins/strict EOF 断言保持。

manager 专项4、shared body6、global metadata17、native metadata6通过。
round2 完整 tunnel 723 PASS / 0 FAIL / 2 原 ignored、upstream access10通过。
round3仅对权限测试诊断断言做 rustfmt 换行，其余662个源码与round2逐一同hash；
生产未变，完整测试证据复用round2。最终权限专项、all-features/all-targets
CLI Clippy -D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux
trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

失败记录保留：round1 权限热更新测试预期403收到502，首次响应类别/正文
未留存，原因未确认。补上诊断后，同一断言在专项/完整回归/最终专项三次
通过，没有放宽权限断言或期限。round2完整测试/构建通过后，格式检查要求
诊断断言换行，已在round3修复并通过。

663个最终冻结源码与worktree一致；core/Cargo未改，既有core证据复用，
不声称重跑。CLI SHA-256：
`8aaa74d395c34091677b5e082029cf95d8d59c7bd4e9603ef0ee82abd8314505`。
全部日志、各轮delta、binary与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-manager-init`。

Ledger/registry/其他共享 owner、private headers、opaque IO/error、SDK及
公开初始RSD、HTTP/框架任务及完整freshness审计继续。128MiB为已准入对象池，
非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push或发布。
