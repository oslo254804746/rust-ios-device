# L02 inline independent deadline timer · 2026-10-18

ConnectionDeadline::run 原初始独立 Timeout 及前后 retry clamp 三处使用
Box::pin，产生未单独准入的 heap payload。本项将原 Timeout<T> inline
保留在原 future 并 pin，clamp 通过安全 Pin::set 原位销毁旧 timer 再替换。
该稳定 API 的本机 Rust 标准库文档和实现已核对；没有 unsafe、额外 Box、
新时钟或替代错误。原 pending Timeout 继续产生原生 Tokio Elapsed。

候选/whole/retry 的绝对最小期限保持：poll 前先检查独立 timer；原工作在
同 Ready poll 返回的 native IO 错误仍优先保留；晚成功不被发布；clamp 后
在返回 Pending 前注册新 timer。Source frame 实际类型尺寸随 inline 存储
变化，已有实际构造器/精确配额测试保持按真实尺寸计算。TaskCell 和 Tokio
私有 runtime 存储仍为独立待审计项，不据消除 Box 声称完整 heap 归属。

新增2项通过：原工作 Pending 后外部收紧期限，超时不再 poll 原工作且返回
原生 Elapsed；同 poll 收紧期限使原晚成功值仅析构一次，同时原 Ready IO
错误保持原指针。实际 native deadline15、manual deadline13、完整 tunnel
735 PASS / 0 FAIL / 2原ignored、upstream access10通过，包含原 fallback/
总期限/late metadata/publication rollback/native joins/strict EOF 断言。
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读
manager1通过；Source/usbmux trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。round1新增测试漏导入Elapsed的编译失败保留，
仅补齐导入后round2全部重新验证，原生产改动及错误断言不变。

668个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`0b9c3e0654f7469f66c4f782efe4b02030bcbb18471ab3d883987e51d533b1c1`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-inline-deadline-timer\round2`。

DeviceGate/其他 owner、terminal/anyhow/private header/opaque error/IO、发现/
SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为
已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与
Q04全部排除保持。仅本地提交，无真实设备、push或发布。
