# L02 terminal cleanup original inline values · 2026-10-18

原 server cleanup 将已取得的 DrainFailure、HTTP 错误、HistoryFailure 与
Elapsed 再封装到 Arc，包含 Box dyn 转换和 Cause unsizing。阶段记录只在
原 parent 的两个 join 分支中使用，最终报告也只移动到返回值。本项保留原
具体类型，完成/取消原 work 后消费 inline mutex 的原阶段值，直接移入终态
对象；standalone terminal drain 同样移动原值，不再创建这些共享 wrapper。

TerminalCleanupFailure 保留具体 native/history 值、原 anyhow HTTP 值和
原 Elapsed。primary/source 仍为原顺序：non-expiry history、non-expiry
native、metadata HTTP、scan history、当前独立 expiry、native/history expiry。
原类型借用代替必定同类型的 dyn downcast，原 class/IO/source/downcast、
Snapshot、完成 flags、metadata-only timeout 的原 Elapsed 返回规则保持。
不克隆原错误、不重新生成 timer、不丢弃尚存错误或把 stop 当作 joins。

新增2项通过：阶段 mutex 到 terminal 再到外层返回对象的移动保持原 HTTP
IO 指针、类别、单次析构和固定 Debug/Display，HTTP IO 仍优先于 native
纯 expiry 与当前独立 expiry；终态 source 在返回前后借用自己存储的同一
原 timer token，类别仍为 Timeout。原 server/caller Drop/堵塞清理、实际
native joins/EOF、旧 scan/history 原指针和总期限测试保留。

终态专项2、server全组8、native CLI105 PASS / 0 FAIL / 1原ignored、完整
tunnel751 PASS / 0 FAIL / 2原ignored、upstream access10通过。严格
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读
manager1通过；Source/usbmux trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。

675个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`faca560f38be239127b209b4b07fa4c8bce92c245dfe52e229563059958b16dc`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-terminal-inline-values\round1`。

外层 anyhow/context allocation、Core stdIO/private wrapper、其他 owner、
private header/opaque IO、发现/SDK原输出、公开初始RSD、HTTP/框架任务及
完整freshness继续审计。128MiB为已准入对象池，非完整heap/RSS；L02不关闭，
主项23/6/L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备、
push或发布。
