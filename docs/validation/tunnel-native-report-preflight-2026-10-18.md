# L02 native report preflight and inline attempt values · 2026-10-18

原 required drain/capture 在取得原错误后新建 Arc<TunnelError>，费用未在
Source 前准入；attempt/Cause/Elapsed 又引入共享分配。本项按原历史规则
只在 Registry 初始化时创建两个实际预付空根，分别保留首个 expiry 和首个
non-expiry。实际 NativeReport body 包含 OnceLock<TunnelError> 和 permit，
与 Self/slots 全部取得原池 grant 后才创建根，先于任何 native ownership。
历史根一经填充便不再修改，封闭 Shared 在 allocation 结束后才析构原错误/
费用；引用审计无该根的 Weak/pin/raw Arc 出口。

record_failure 保留原 per-owner observation、additional counter 与 IO 优先
规则：首次对应原错误移入已付费根，返回同一 root；后续新不可变报告作为
Current 原值内联在本次 attempt。不会复用旧 joins flags、替换原错误或在
取得错误后申请预算。AttemptEvidence 原优先级不变，返回时移动首个原值；
DrainFailure 内联保存原 Elapsed，借用同一 token 或原 TunnelError 驱动
primary/source/class/downcast。原 pure expiry 后 IO/JoinError 优先、实际
joins_complete 认证、旧报告冻结、native handle/原 IO 指针与绝对时钟保持。

Registry 结构预算计入两个实际 body；测试重配只允许唯一且未填充的根，
先取得新整笔费用，再原位换入 grant 退旧池，原 slots/root 地址保持。原
slot backing 差一字节测试按真实新成本补齐，仍在同一目标边界拒绝，无
padding、免费 sidecar、额外 worker 或新池。Core stdIO/private wrapper、
runtime TaskCell、server/terminal anyhow 等独立边界未据此计为完成。

新增3项通过：Self/report/slot 的零与差一字节拒绝先于 backing 分配，精确
两空根在 Registry 结束后分别退款；8线程捕获根保持原 IO/root 地址及 body
到最后 owner；实际 worker 在闭池零额度下取得 pending 报告、原独立 timer，
之后 current joined 报告位于原 DrainFailure 对象内，原 cause 同指针、旧
pending/history 快照冻结，实际 joins/task baseline 完成后准确退款。

专项3、retirement38、native CLI105 PASS / 0 FAIL / 1原ignored、attribution9、
完整 tunnel749 PASS / 0 FAIL / 2原ignored、upstream access10通过。严格
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读
manager1通过；Source/usbmux trap0，host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。round1 Snapshot 分类 getter 仍借用旧 Arc payload
导致编译失败，已改为借用同一原 TunnelError，未给存储 wrapper 伪装 Error；
round2全部重新验证，失败日志保留。

674个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`cb9002da101e1375a063d8e182d001bf8dcdd3aff9e3b655d292df9d2019af98`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-report-preflight\round2`。

其他 owner、terminal/anyhow/private header/opaque error/IO、发现/SDK原输出、
公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为已准入对象池，
非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push或发布。
