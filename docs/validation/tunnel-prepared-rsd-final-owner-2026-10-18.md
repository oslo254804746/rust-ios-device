# L02 prepared RSD sealed final owner · 2026-10-18

管理器原刷新目录直接导出 Arc<PreparedRsdCatalogue>。body fee 位于 T 内，最后
strong 结束即退款，但导出的 Weak 仍可保留原分配。本项新增 PreparedRsdOwner，
封闭原 Arc，不提供 raw/Weak/mutable/pin 接口；所有 Clone 共享原 payload，每个
Drop 均使用 Arc::into_inner，使分配结束后再销毁原目录、字段及费用。没有复制目录、
额外 body allocation、独立短寿命 sidecar 或私有 Arc header 大小估算。

prepared finish 拆出原 selection/fee 安装步骤，同一个已准入 catalogue 原值选择
legacy Arc 或 sealed owner 结束方式。新增 prepare_owned_existing_rsd_at 及显式
identity 入口，和 legacy 函数共用 Source、checked protocol、原 direct/proxy、
原 absolute deadline 与 selection receipt。旧 prepare_existing_rsd_at/identity_at
以及 ConnectedDevice 的 legacy raw Arc 接口保持兼容，其 raw Weak 边界仍存在。

NativeSource refresh、freshness Metadata、设备 current cache、registry proof、
同步交换与回滚均改用 sealed owner，保留 ptr_eq 身份。此任务本地新增的
OwnedConnectedDevice 刷新 getter/swap 使用 sealed 类型；其私有 current slot
与 legacy ConnectedDevice raw slot 分离，current_rsd_catalogue/服务 resolver
选择实际 current catalogue，不把可用目录改成 None。runtime/UID 检查在 swap 前
保持，初始目录继续为独立历史观察。Source 原 frame 在 metadata 准入前销毁，
原 global/publication body/field 费用规则保持，没有重复收费或第二原生 lease。

新增3项通过：目录 body/rows/UID/features/selection 原指针在 input/root drop 与
关闭原池后保持，最后 owner 全退款；8线程相同原 payload 同步释放只退款一次；
公开 owned connection 拒绝错误runtime、current getter 指针/owner 身份、当前刷新
优先于初始服务解析、关闭配额不重新发现，以及 export 在设备释放后继续持有费用。
最后一项使用 scripted H2/channel-only handle，不声称实际 native worker 测试。
原 CLI freshness21项用相同实际 H2/XPC 协议改读 sealed 输出，原 stop/epoch/late
swap rollback/原 typed cause/实际 userspace worker joins 和严格 EOF 断言保留。

新跑 Core/CLI strict all-features/all-targets Clippy -D warnings、prepared-owned3、
XPC210、activation9、proxy4、prepared6、existing18、Core tunnel131、无mdns新增3/
proxy5、无默认/仅tunnel编译通过，实际cfg warnings保留。完整 tunnel751/0FAIL/
2原ignored、完整CLI1731/0FAIL/13原ignored、build/fmt/diff及实际只读manager1通过。
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。

本轮21个串行门禁通过；679个冻结源码无漂移，15处源码差异。本次Core已改，
上述为新验证。CLI SHA-256：
`1d37bbe23de552ce06afcf15a8a7d9e68efb0d7644d2ca2812509c059df05dfe`。
日志、source delta、受测binary、current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-prepared-rsd-final-owner\round1`。

standard Arc/Bytes 私有header、legacy公开raw prepared/initial SDK接口、selected
service metadata副本、发现/其它SDK原输出、opaque error/IO/框架task/allocator及
完整freshness仍需审计。128MiB为已准入已知对象池，非完整manager heap/RSS；
L02不关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部排除保持。无真实设备、
push或发布，仅本地提交。
