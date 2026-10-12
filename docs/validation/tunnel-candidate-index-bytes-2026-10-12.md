# L02 candidate reconciliation借用索引准入 · 2026-10-12

首次388项门禁后核查出默认HTTP refresh fixture使用NativeSource的隔离缺口，已
改为NoDevices并串行重新验收。原测试/build/CLI日志以 `before-isolation-*` 保留；
不能将首次全组结果用于认证无真实发现。具体修正与NOT_OBSERVED边界见
[fixture隔离更正](tunnel-fixture-isolation-2026-10-12.md)。生产NativeSource及原localhost
native joins/严格EOF断言保持。

承接 `53981b90c09f1f6d49df0eb69bea7c848122279b`，生产failure/retry reconciliation
的临时HashMap与Route host克隆改为借用原candidate身份的已准入索引Vec。
按实际capacity从同一128MiB bank收费，准入在failure writer等待/表清理之前完成。

索引按UDID和原位置排序，二分查找返回原Route引用和原native ID；候选重复时仍
使用原map的最后记录规则。原candidate持有全部key/Route/Ticket根，borrow约束保证
索引跨异步writer等待时这些根仍有效；本项不复制任意host、error或诊断文本。
索引对象退出/caller Drop归还其backing charge；拒绝不改原failure或retry表。

新增5项门禁通过：Route对象/host buffer身份及spare capacity/native ID、重复key与
空/NUL/UTF-8 selector、精确容量/一字节拒绝/refund、空索引零bank，以及生产refresh
索引拒绝保持原backoff version/slot/identity/时间/计数且Source连接零调用。
最后gate/state/保留的预算owner释放后，全额恢复共享bank。受影响tunnel388通过、
0失败、2原ignored；原error/epoch/native joins/strict EOF门禁未放宽。严格Clippy/build、
fmt/diff与readonly CLI1项通过；readonly策略不启动Source发现，独立localhost trap
零连接。host runner为5 PASS、0 FAIL、22 NOT_RUN，real_device_executed=false。
browser和显式L04 ignored保留，不称全量Rust/SDK重跑。精确来源由本项summary登记。

本项只计此reconciliation索引backing，不是整个candidate/manager heap的完成声明。
原candidate Vec/owned fields、seen/attached/manual/initial及failure/retry/scan/native
历史backing、manual route副本、watch/Arc和原error payload完整创建来源/字节准入
仍缺交付；完整freshness/刷新/复用/重启/manual epoch也继续。原错误对象、主/清理
原因、严格EOF和原native joins断言保持，禁止用计数/Drop替代完整字节合同或ACK。

L02仍未关闭；账本23开发交付、6缺开发、L02待完整收口，有效30，Q04全部排除。
Rust/Cargo串行，无子agent、真实发现/设备操作、push/发布/部署。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-candidate-index`。
