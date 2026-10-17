# L02 original candidate input text · 2026-10-17

原 Input::owned 在接收已经复制的 UDID/host String 后才准入，不能证明这些
原缓冲区的创建前预算。本项将三处生产候选路径（manual、planned mobdev2、
automatic paired）全部改为借用 RouteRef/UDID；不再先 clone host 或 to_owned
UDID 以构造临时输入。旧 moved-input 构造器仅留 cfg(test)，没有生产绕路。

Input::copied 先验证 selectors 并取得原 body、Source route clone 和 outcome
key 的预留，再对每段实际文本使用 exact_backing<u8>(len) 创建前准入。原 Vec
直接成为 String，同一指针、精确 capacity，不再复制。每个 field 的原费用在
复制前合并进同一个 Input permit，字段位于 permit 前，最后 Input/原错误
owner 析构文本后才退款；部分失败的 local storage 也先绑定，保证已创建文本
先析构再退款。未新增 Input 字段或改变结构大小，总费用保持原范围，无重复
收费。USB 原 snapshot String 转移及 raw fee split 保持，不新增该字符串。

原 Source clone 与 prepared failure/success key 的独立预留继续保留；没有
因本次原文本修复删除这些费用。原 source_route 的普通 clone/事后 capacity
校验、SDK/发现输出与 Arc/anyhow 私有分配仍是独立边界，本项不声称覆盖它们。
原 USB/paired 选择、manual 过滤、批次回滚、epoch/绝对期限、原错误与 native
lease/join 语义保持。

candidate_input 专项 7 PASS：新增 3 项覆盖所有 route 家族、None/空 host/UTF-8
原 allocation hook/预付和指针、零/最后一字节/部分字段拒绝及退款、selector
拒绝零分配、最后共享错误 owner 与原 typed IO pointer。原错误 context 测试
也已改走生产 copied 构造器；USB 原指针/不重复收费测试保持。完整测试同时
验证真实 manager 的 paired 输入批次拒绝、保留 USB/旧 backoff、Source 原错误
跨 clear/drop 及原 native join/EOF；本轮无需放宽或调整旧精确预算断言。

最终完整 tunnel 665 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；
CLI all-features/all-targets Clippy -D warnings、build/fmt/diff 通过。实际只读
manager 1 项、Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。645 个冻结源码无漂移；core/Cargo 未改，既有 core
证据复用，不声称重跑。受测 CLI SHA-256：
`21acbd205488ee63622c3f0f09a3fb8bf48ece4d71b9ef2e4486812eb58912f5`。

日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-candidate-text\round1`。

Source 临时 route/clone 与 discovery snapshot/设备元数据创建端、SDK future、
初始公开 RsdHandshake 目录逃逸、config/client/URI/received/relay/framework/
private IO，以及完整 freshness/刷新/复用/重启/manual epoch 审计继续。128MiB
为已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全
审计、有效 30 和 Q04 全部排除保持。仅本地提交，无真实设备、push 或发布。
