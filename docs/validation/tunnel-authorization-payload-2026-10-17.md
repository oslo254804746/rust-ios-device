# L02 upstream Authorization payload · 2026-10-17

上游header原先创建并增长Zeroizing prefix String，然后HeaderValue::from_str
再复制。现在HTTP listing与remote WebSocket request共用manager builder：
按实际prefix+token字节数先准入u8 Vec，再写入两个借用slice，消除临时String。
锁定http1.4.0的from_maybe_shared(Bytes)直接保留原Bytes；bytes1.11.1的
from_owner固定owner并借用其slice，原始payload指针不变。

HeaderValue/request/clone最后owner持有原grant。Payload析构先zeroize整个
实际Vec，再由AdmittedVec释放bytes、归还费用；无效HeaderValue构建失败同样
清零/退款并保留原typed InvalidHeaderValue。set_sensitive保持，错误/Debug
不输出token。Bytes私有owner Box、HeaderMap/HTTP序列化/TLS/request/URI及
allocator内部存储仍是独立边界，不由本项payload费用覆盖。

origin/transport/root校验仍先执行，然后才调用builder；无token不调用builder。
builder拒绝保留原typed cause/指针，不把ResourceLimit改成凭据缺失或成功。
原CA/hostname/expiry、no-proxy/no-redirect和独立manager凭据规则保留。

新增5项manager测试覆盖真实allocation hook、HeaderValue/Request move和
clone原指针、最后owner清零先于退款、差一字节拒绝无payload分配、无效header
原typed错误和清理、实际localhost认证HTTP及配额拒绝时请求0、WebSocket
request/最后header离开client/input/manager后的独立持费。WS新配额测试在
request构建边界，未声称实际握手拒绝；原双manager TLS/WSS转发全组仍通过。
新增1项access测试证明目标不匹配先于builder、无token不建payload以及原
builder IO错误指针不变。原7项access合计含此新增项。

专项5PASS；完整tunnel603PASS/0FAIL/2原ignored、upstream access7PASS；
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1
通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。626个冻结源码无漂移，core/Cargo未改，复用此前
fb89530 core诊断7/8及strict证据，不冒充重跑。CLI SHA-256：
`4e8ed04fa8e404b13520630d86598d45d9134eb03b98e9d68d974ba9d9ab2fcd`。

日志/冻结清单/受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-authorization-payload\round2`。
round1保留新增fixture错误构造ClientToken的编译失败，修正为原Zeroizing字段
类型后round2全部通过；原fixture/安全约束未删除。

client root/config/credentials原存储、URI/framework/WS Binary/sink/private IO/
error/Arc/allocator/TaskCell、Source/candidate创建、initial公开RSD目录及完整
physical freshness/刷新/复用/重启/manual epoch/选择/prune继续。128MiB不是
完整manager heap/RSS上限。L02不关闭，23交付/6缺开发/L02待完整审计、
有效30与Q04全部排除保持。仅本地提交，无真实设备、push、发布或部署。
