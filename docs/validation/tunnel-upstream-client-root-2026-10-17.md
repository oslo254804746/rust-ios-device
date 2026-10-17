# L02 shared upstream client root · 2026-10-17

UpstreamClient原derive(Clone)每次复制root String；Upstreams::client还先创建
一份normalize结果。现在初次构造把原String直接move进Arc<String>，保留其
原data指针，client克隆仅共享原root。未用Arc<str>另复制原文本。对不可变且
已验证registry的完全相同canonical key直接取原client；别名、非精确输入和
匿名fallback继续原normalize/lookup/new路径，原TLS/token绑定仍在header
构建前独立复核。

快速查找先执行与原normalize相同的2048-byte原始输入上限。锁定url2.5.4/
idna1.0.3的DnsLength::Ignore允许Unicode短域名规范化后超过该限；固定fixture
验证这种key虽能由原配置流程建立，直接传入超长canonical URL仍按原规则拒绝，
原限内Unicode别名行为也保持，未把fast hit变成校验旁路。未更改原URL协议、
path/query/credential、loopback或root选择规则。

新增3项access回归覆盖原String指针move/clone最后owner，canonical/别名/
多registry引用同原root和token，registry/input销毁后仍可生成正确敏感header，
无效URL拒绝与IDNA长度边界。原实际HTTP/TLS/WSS/双manager转发验证保持。
初始tradeoff是新增一个私有Arc容器，没有据此声称CPU/RSS改善；初始config
root/map/key/Arc存储仍未接入global bank，本项仅消除重复请求文本副本，不
宣称完整client/config费用已付。

最终完整tunnel603PASS/0FAIL/2原ignored、upstream access10PASS；all-feature/
all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；
Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
626个冻结源码无漂移。core/Cargo未改，复用此前fb89530 core诊断7/8及strict
证据，不冒充重跑。CLI SHA-256：
`242c8765667f82ab3638b2252a0c98dd5848b72154d50b1650113008145a3bdd`。

最终日志/冻结清单/受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-upstream-client-root\round2`。
round1原指针/别名2项通过；补长度先验与第三项后round2完整门禁通过。

初始config/client/credential、URI/framework/WS Binary/sink/private IO/error/
Arc/allocator/TaskCell、Source/candidate创建、initial公开RSD目录及原完整
physical freshness/刷新/复用/重启/manual epoch/选择/prune继续。128MiB不是
完整manager heap/RSS上限，L02不关闭，23交付/6缺开发/L02待完整审计、
有效30和Q04全部排除保持。仅本地提交，无真实设备、push、发布或部署。
