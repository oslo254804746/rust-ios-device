# L02 shared route destination owners · 2026-10-16

ManagedTunnel原RouteIntent在Source前从desired/输入复制，publication再次复制，
live shortcut再复制到新的RetainedRoute；先前snapshot grant不覆盖这些独立
文本创建。本次ManagedTunnel与连接local都持有Arc<RetainedRoute>：首次按
真实u8 Layout准入/分配host，payload body在Arc::new前准入；已有desired的
自动恢复直接共享原Arc，publication、manual desired和live shortcut均共享同一
原文本。Usb/Wifi保留实际选择host，Mobdev2保留原rediscovery filter，None与
Some(empty)及live shortcut忽略新请求transport的规则保持。

data先于两个grant析构，最后table/desired/snapshot owner控制退款；不依赖
较短的调用方或device sidecar。Arc私有header/allocator不由payload费用覆盖。
manual_shared在原checked_replace期限/权限/epoch围栏中校验原bank身份，
异池拒绝保持原version/watch/费用；回滚保持原publication与desired Arc。
新增RouteAdmission context保留原cause，并阻止新pre-Source配额拒绝进入默认
discovery fallback。ManualRequest借用仍存活的query filter，移除独立String
副本；Source的原Route及observed_route副本仍是另外的待接线边界。

独立RetainedRoute已在global bank持费，shared native/table metadata不重复
收取其文本。原单独published ceiling仍计算desired文本；device/observed
permit的原native joins寿命不变。移除table且无desired snapshot时路由body
确实销毁并单独退款，native device/input仍由原joins持费；新增Weak断言核对
这一实际分界，原strict Ok(0) EOF、worker回收和退款金额断言保留。

新增5项覆盖actual text allocation hook/精确容量与最后snapshot、body/text
分别差一字节的创建前拒绝/退款、None/empty、真实ensure的零Source/USB/
paired且不fallback、live shortcut/自动恢复同Arc与同text指针、foreign-bank
拒绝时原version/watch/费用。既有moved-input fixture仍保留原指针/容量及
transaction rollback测试；生产路径使用新的创建前准入与共享接线。

最终完整tunnel592PASS/0FAIL/2原ignored，上游访问6PASS；all-feature/all-target
CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；host5PASS/
0FAIL/22NOT_RUN，Source/usbmux trap0，real_device_executed=false。621个
冻结源码无漂移。core/Cargo未改，沿用既有fb89530 core诊断7/8及strict证据，
不冒充重跑。CLI SHA-256：
`beeeb9bd36d267b26abe798dba8f58cb01524d8c7f359e2b07940c54491986cc`。

最终日志/冻结清单/受测binary/current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-shared-route\round4`。
round1保留旧fixture类型比较编译失败；round2新增4项通过，full586/5FAIL为
native fixture漏算新route body。补exact size_of公式和Weak析构断言后round3
native6及full591通过；随后Clippy揭示旧bank字段未读，接线实际same-bank检查
并新增第五项后，round4全组通过。失败日志和原金额/EOF验证均保留。

observed/Source/candidate文本创建、URI/header/client/config credentials/policy/
auth/framework/WS/private IO/error/Arc/allocator/TaskCell、initial公开RSD目录
逃逸及完整physical freshness/刷新/复用/重启/manual epoch/选择/prune继续。
128MiB是已收费自有对象池，不是完整manager heap/RSS；L02不关闭，23交付/
6缺开发/L02待完整审计、有效30和Q04全部排除保持。仅本地提交与隔离host
fixtures，无真实设备、push、发布或部署。
