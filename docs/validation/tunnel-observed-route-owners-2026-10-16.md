# L02 observed route destination owners · 2026-10-16

Source前原observed_route = route.clone()创建第二份普通host String，后续metadata
费用不构成其创建前准入。现在Observed在有candidate Input时共享原Input.route，
校验同bank和同原Route值后只clone Arc；无Input的手动路径按真实u8 Layout
先准入再复制，保持Usbmux/Usb/Wifi/Mobdev2、None/Some(empty)及实际发现host。
desired Mobdev2 rediscovery filter与observed实际host仍分开，未把filter当地址。

owned route先析构原String再归还grant；shared input随最后observation/input/
native/table/error owner退款。原Source接收的route和其独立预留费用不与本次
observed owner混淆。publication ceiling仍计算两类route文本；global metadata
减去已独立持费的observed文本，避免同池重复收费。新admission marker保持原
Resource cause和pre-Source拒绝，不进入默认discovery fallback。失败时原
publication backing可保留复用且继续持费，未误记为泄漏或已退款。

生产reconciliation继续借用原Route做同样的完整paired-view匹配；不完整/较老
发现、USB/paired选择、地址变化和prune规则保持。只读freshness fixture取值
时显式clone raw Route，生产不再创建该独立副本。Source本身初始Route和
candidate输入创建边界仍独立待核查，不能由本项推导全部文本已接线。

新增6项覆盖实际allocation hook/精确容量/离开原输入，差一字节零分配并
保留typed resource、所有route变体，零额外配额共享原Input地址/最后owner，
异池或不同observed值不替换原观察；实际ensure在目标observed差一字节时
HTTP/Source前拒绝、USB/paired调用0、native空且部分destination精确退款；
实际scan publication同原Input对象，并移出原observation后通过正常manager
清理证明其持费寿命独立。原native metadata/joins/strict Ok(0) EOF全组保留。

专项6PASS，完整tunnel598PASS/0FAIL/2原ignored，上游访问6PASS；all-feature/
all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；
host5PASS/0FAIL/22NOT_RUN，Source/usbmux trap0，real_device_executed=false。
623个冻结源码无漂移。core/Cargo未改，沿用此前fb89530 core诊断7/8与strict
证据，不冒充本轮重跑。CLI SHA-256：
`b9300e3de5617dcbee5b4de383f11565efb5c85f00fc6d2141cac84b15ee4b1b`。

日志/冻结清单/受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-observed-route\round2`。
round1保留新增fixture误把opaque Removed当行访问的编译失败；修正为直接
移出原observation、正常cleanup原row/input/native owner后，round2全部通过。

Source/candidate创建、URI/header/client/config credentials/policy/auth/framework/
WS/private IO/error/Arc/allocator/TaskCell、initial公开RSD目录和原完整physical
freshness/刷新/复用/重启/manual epoch/选择/prune审计继续。128MiB仍是已收费
自有对象池，不是完整manager heap/RSS上限；L02不关闭，23交付/6缺开发/
L02待完整审计、有效30及Q04全部排除保持。仅本地提交，无真实设备或发布。
