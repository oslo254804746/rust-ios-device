# L02 manager JSON wire ownership · 2026-10-16

原 manager handler 返回 Json<T>，Axum 在 IntoResponse 中创建 BytesMut(128)
并可扩容；原 handler/local metadata 费用不跟随发出的 body。当前改动把所有
manager 自有 JSON handler 接到 Replies，未把只读 health/ready、鉴权/ticket/
framework rejection 等独立响应边界算成已覆盖。

成功回复同步对同一冻结 value 做两遍 serde_json::to_writer：第一遍只计算 checked
字节数，随后按真实 Layout 准入并创建精确 u8 backing，第二遍 capped writer
禁止增长并核对实际长度。不存在新增本地 1MiB cap；原 MAX_LIST_BYTES 仍仅限制
upstream input。Vec 与 grant 在同一个 owner，bytes 先于费用析构；转为
Bytes::from_owner 再交 Body，handler/Body 析构后发出的 frame/clone/slice 仍保留
原 grant。Bytes 私有 owner header 和 HTTP 组件私有布局未因此获得完整准入。

授权后的工作开始前预留2048字节固定诊断缓冲区。manager 的原生操作错误直接
序列化进这份已有存储，不在已取得错误后再向池申请 data bytes；保留原 HTTP
status、ErrorClass、phase/schema/effects=unknown 与 reviewed 标记，避免 rejection
middleware 重建 Json。诊断 envelope 仅使用固定 phase/受控枚举及借用的安全
diagnostic formatter，不建外层 String/Value 图。redaction 的内部 hex String
仍属单独缺口，不能把 borrowed collect_str 等同于零分配的完整错误路径。
诊断准入自身拒绝使用 static Bytes body，不递归申请另一诊断 owner。

GET/DELETE 先检查目标权限再占用池；未授权的有效或无效 selector 都不会通过
response quota 遮盖 scope403。授权调用的 selector400 规则保持。简单 mutation
的成功 body 在动作前构造；remove 两种 bool 回复均提前准备。未知的 start 成功
record 只能取得后序列化；若后续成功输出配额拒绝，返回明确 resource_limit/
effects=unknown，已成功的 tunnel 仍由原 owner 管理，不宣称设备动作被撤销。
已取得的 native Err 始终由预付诊断回复保留其类别，不被后续 quota 原因替换。

生产接线含本地/联邦列表、upstream GET/add/remove、单设备 GET/delete/start、
refresh/epoch/shutdown/policy reload、connect 错误和旧 GET 通知。JSON Content-Type、
既有字段/可选项省略及固定 envelope 字节顺序保持。upstream GET 直接借用稳定
BTreeSet；本地过滤在既有 Vec 原地 retain，未新增引用数组。列表/record/Value
的输入副本和 federation graph 本身仍 MISSING，须在创建点另行接线，不能从
wire data owner 推导其覆盖。原权限复核/revocation、network deadlines 未改变。

新增回归覆盖最后 emitted slice 退费、one-byte-short 零分配、实际分配点已有
grant、第二遍增长拒绝/退款、零剩余池原 ConnectionReset(disconnected) 类别、
static 构造拒绝、大于1MiB输出/Unicode/省略字段、实际 loopback Source0 和 epoch
动作前拒绝，以及低配额 scope403 与固定 envelope 字节顺序。
round1 537PASS/1FAIL/2原ignored，唯一失败是新测试误期望io而非原disconnected；
生产分类未改。独立审查发现的 GET/DELETE 权限时点已修复，失败日志保留。
round2 完整 tunnel540/0FAIL/2 原 ignored 通过，十项新增全部通过。
Clippy 随后仅提示6个回复方法的 result_large_err；最终只在 Replies impl 添加
有说明的局部例外，避免在原生失败后再独立 Box 响应。其余 -D warnings 保持。
round3 的 runtime/test 源码与 round2 完全一致（reuse-proof 逐字节去掉该属性及
说明后核对哈希），复用540证据，不冒充重跑。最终 CLI all-feature/all-target
Clippy、build/fmt/diff、只读 manager1 通过；Source/usbmux trap0，host5PASS/0FAIL/
22NOT_RUN，real_device_executed=false。冻结605个 build-source 无漂移；core/
Cargo 哈希未变，复用原core201/131等证据。受测 CLI SHA-256：
`15340f050397b0c64e7aff775232883e6abc9aea805b66521131ac459ccb84ad`。
完整日志、受测binary、freeze/reuse/source-delta/current及提交后rawGit proof在round3。

证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-json-wire-owner`。

record/Value graph、upstream received input、WebSocket sink、鉴权/framework、
private error/IO/Arc/allocator/TaskCell、initial 公开目录及完整 freshness 等仍继续。
128MiB 非完整 manager heap/RSS，L02 不关闭；23/6/待全审计、有效30、Q04全部
排除不变。仅隔离 localhost/duplex/只读Source0 smoke，无真实设备或 push/发布。
