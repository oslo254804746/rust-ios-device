# L02 formatter-owned fixed hex · 2026-10-16

DiagnosticId 与 DiagnosticError 的 formatter 原来分别通过 encode_id/error_id
返回 hex::encode 创建的16字节 String。它们可在已预付诊断 reply 的路径中出现，
所以借用 collect_str 不代表 formatter 自己没有独立 heap buffer。

两个私有 helper 现在返回 DigestHex([u8;16])。SHA-256 finalize 后原前8字节
直接写入固定数组，Display 借用 ASCII 写入已有 formatter。锁定的 hex0.4.3
encode_to_slice 源码已核查，仅按索引写目标 slice，无新容器。没有新增 grant、
独立 owner 或后置申请；公开 API 和 process-local salt/entropy fallback 不变。
原 id:/error: 前缀、lowercase16位 hex、原错误类别/IO kind、Display/source chain、
4096哈希字节与8个source上限完全保留，fmt::Error 继续向输出端传播。
任意底层 Error::fmt 可自行分配，其私有 payload 和第三方实现仍是独立边界。
本项仅消除本 formatter 的两个已知 String，不据此宣称完整错误路径零分配。

新增两个回归：固定key的Unicode/NUL标识和两层原错误链，与改动前算法的
独立SHA-256参考字节一致；原source指针及writer拒绝保持。原循环/字节上限、
普通/pretty Debug 和标识脱敏回归保持。all-feature core redaction7PASS，
no-default-features8PASS（含原fallback TunnelInfo）；最小feature另有7条既有
feature-off dead-code警告，未改动对应代码。core和CLI all-feature/all-target
Clippy -D warnings、完整tunnel550PASS/0FAIL/2原ignored、build/fmt/diff、实际
只读manager1均通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。609个冻结build-source无漂移，唯一源码差异为
core redaction.rs，CLI/Cargo未变；不把历史core201/131等或完整CLI1480当成本轮
重跑。受测CLI SHA-256：`193ff2a092ab2a11f58420543267e70519a05496fd80ee3c9e099121b4ce9298`。

完整日志、受测binary、source-delta/freeze/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-diagnostic-hex\round1`。

联邦Value/map/key与received input、WS Binary/sink、鉴权/framework、其他private
IO/error/Arc/allocator/TaskCell、initial公开目录和原完整freshness等继续。
128MiB非完整manager heap/RSS；L02仍未关闭，23交付/6缺开发/L02待全审计、
有效30、Q04全部排除不变。仅本地提交和隔离localhost/duplex/只读Source0 smoke，
无真实设备、push、发布或部署。
