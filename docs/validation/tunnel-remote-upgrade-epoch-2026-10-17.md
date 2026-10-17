# L02 remote proxy global epoch · 2026-10-17

原remote on_upgrade只监听principal.revoked。真实localhost leaf listing、WS
转发和原TCP echo peer复现：本地stop(remote UID)返回false且远端流继续传输，
但root clear/shutdown两项均未关闭旧proxy，红灯结果[false,false]保留。

新增只读GlobalEpoch复用Lifecycle原shutdown watch与原Arc generation，既不
创建device key/coordinator，也不spawn新任务或添加短流期限。远端selection、
原WS dial及已升级relay绑定同一个原global view；selection完成后再次检查，
dial成功后同步commit拒绝已退休handoff。clear、terminal close及Lifecycle
sender Drop使旧view失效，后续work不poll。原ready错误在同一poll跨clear
仍直接返回原typed cause/指针，成功结果则必须再过handoff fence。

远端流不挂本地device ticket，所以本地stop(A)不会把外部远端A当成本机运行时
取消，也不向远端发管理请求。原2s listing与15s WS dial、TLS/hostname/CA、
token/header zeroize、redirect/URL/hop/interface、原错误类别和权限撤销规则
保持。本地连接继续原ticket+publication双重寿命，不弱化manual live语义。

新增5项：真实两跳流先证明local stop仍可传输，再clear/shutdown关闭root
stream、leaf stream及原TCP strict Ok(0) EOF，peer真实handle await；全局
snapshot不创建device/不收费新key；退休view零poll且新view可继续；pending
work在clear/close/owner Drop实际析构；ready原IO跨clear类型/指针不变；
ready成功跨clear不能commit且原模拟stream销毁。通用work Drop不等同于私有
HTTP连接driver join，本轮未声称已验证未完成listing/握手的全部底层IO回收。

专项5PASS；完整tunnel629PASS/0FAIL/2原ignored，upstream access10PASS；
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1
通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。634个冻结源码无漂移。core/Cargo未改，复用此前
fb89530 core诊断7/8及strict证据，不冒充重跑。CLI SHA-256：
`14b79233ff7bde8097a424c1e09686c85ee32c353405eaf59eed85621106a956`。

最终日志、冻结清单、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-remote-epoch\round1`。
red目录原两项false复现保留，修改后同一冻结源码完整通过。

待继续：pending listing/握手原wire回收，原框架upgrade实际joins/TaskCell，
received/relay/private buffer/IO/error/Arc/allocator，初始config/client/credential、
URI、Source/candidate创建、initial公开RSD目录和全部原physical freshness/
刷新/复用/重启/manual epoch/选择/prune全审计。128MiB非完整manager heap/RSS；
L02不关闭，23交付/6缺开发/L02待完整审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push、发布或部署。
