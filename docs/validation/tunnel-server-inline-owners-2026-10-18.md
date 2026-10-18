# L02 server inline future and phase storage · 2026-10-18

server::serve 原 scanner/HTTP 两个 Box future 及 CleanupPhases 的 Arc mutex
payload 未单独准入。本项将原 HTTP future inline pin，scanner 保存在 inline
pinned Option；停止时 Pin::set(None) 在原位销毁实际 scanner，仍先于历史
读取和清理。原控制流中的 clock 创建、notify_shutdown 顺序保持，不能用
只 Drop 一个 Pin 引用代替实际析构。安全 pin/Option API 无 unsafe 或新任务。

两段 tokio::join 清理分支属于同一 parent future，改为借用 inline StdMutex
阶段记录，保持原同步写入、原错误持有和终态 flags。HTTP server_finished
及后续 drain await 保持，不重新 poll 已完成的原 future。移除这三处自有
payload 分配，不改变原 Source、permit、clock、history、primary/cleanup
或 native joins 规则。parent TaskCell、Axum 私有任务与终态错误的私有存储
仍是独立待审计项，不据 inline 存储声明完整 heap/RSS 或框架任务已 join。

新增实际 Source frame 测试：scanner 已进入 Pending，signal 触发停止，
parent 的 cleanup 继续被表写锁阻塞；在表解锁和 parent 完成之前，原 frame
已仅析构一次、connect/failure slot 许可准确归还且 native registry 为空。
随后原 parent 正常完成且 listener 关闭。原 caller Drop、HTTP shutdown、
信号原 IO、cleanup 总期限、旧 history、实际 native joins/strict EOF 测试
均保留。Linux 实际 SIGINT 项在 Windows 不运行，不声称已验。

Server全组8、native server2、native scan outcomes7、完整 tunnel745 PASS /
0 FAIL / 2原ignored、upstream access10通过。all-features/all-targets CLI
Clippy -D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux
trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

673个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`267dda4f4b0313ae663275e5be46610ea5dbfea332db762a197e26dcaaf76a79`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-server-inline-owners\round1`。

native report/其他 owner、terminal/anyhow/private header/opaque error/IO、
发现/SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB
为已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30
与Q04全部排除保持。仅本地提交，无真实设备、push或发布。
