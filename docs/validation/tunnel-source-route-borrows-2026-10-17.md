# L02 Source route borrows · 2026-10-17

原候选 Source route 使用 String::clone 后才检查 capacity，手动 USB/Wi-Fi
路径也先复制临时 host，再创建持费的 Observed。本项将完整生产路径改为
RouteRef：手动 selector、paired fallback 及 scanner 均借用原字段，manager
创建持费的 Observed/desired 后，Source 从该 Observed 的原 route 借用 host。
NativeSource 直接把原 &str 传入 SDK；生产没有这两种临时 String 分配。
Source 的原具体 F/输出 Box 准入、native owner 和原 typed cause 保持。

Source::connect_owned 全部定义/fixture 覆盖接入借用视图。旧 connect 入口仅
cfg(test)，需要 owned Route 的旧 fixture 在该测试边界显式转换；生产没有
该转换。Input::source_route 返回原字段视图，在零余量下也无需新缓冲区；
原 Source 文本 allowance 暂作保守预留，不声称它是另一段实际 allocation，
prepared outcome key 的独立 UDID 预留仍保留。

Observed/RetainedRoute 接收借用视图，保持原 bank、family/text 一致性检查和
Mobdev2 desired filter 与实际 observed host 的区别。Observed 构造、Source
构造及原 retry 准备在同一 result async 作用域内，Source 借用其局部原 owner；
依然先完成 Source 构造准入，再读取 manual retry 版本/启动 attempt。原 Source
Ready cause 和销毁后输出准入顺序保持，Source 显式销毁后才能将 Observed 移入
发布记录。原 attempt.finish/recovered、绝对 deadline、authority、epoch 和
native lease/join 逻辑保持，没有新增任务、Box 包装、公开 SDK API 或时钟。

新增 2 项专项通过：真实 manager 的 USB/Wi-Fi/默认 paired fallback pending
Source 使用实际已准入 Observed 原 host 指针，caller Drop 时仍可读取原 host，
析构后准确退款，无 native owner 遗留；candidate 的原 host pointer 和 typed
IO pointer 跨 Source Ready、原 retry receipt、manager clear/drop 保留，最后
receipt 才释放 Input 和费用。既有 candidate/Observed 测试也改为核对原借用
指针，无新目的地；生产 NativeSource 四种 route 构造测试仍不 poll 真实设备。

首轮专项 1 PASS / 1 FAIL：新 fixture 将默认 Mobdev2 fallback 误算为两段 host
目的地。原自动 desired filter 为 None，只应分配 observed host；explicit
USB/Wi-Fi 才保留 desired 和 observed 两段。修正测试期望为 1/2，生产 filter
语义不改。首轮日志保留；最终专项 2 PASS，原精确预算断言没有放宽。

最终完整 tunnel 667 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；
CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、实际只读 manager
1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。646 个冻结源码无漂移；core/Cargo 未改，既有 core
证据复用，不声称重跑。受测 CLI SHA-256：
`e5cf4b58bba6e180d6736d9ffc15df777eb50928993be1a1a9f628d9fa982b8c`。

日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-source-route\round2`。

prepared outcome key 普通 clone/事后校验、发现/SDK 原输出与设备元数据创建端、
初始公开 RsdHandshake 目录逃逸、HTTP/URI/client/config/framework/private IO、
原 framework joins 和完整 freshness/刷新/复用/重启/manual epoch 审计继续。
128MiB 为已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/
L02 待全审计、有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
