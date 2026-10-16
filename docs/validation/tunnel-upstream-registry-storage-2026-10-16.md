# L02 registered upstream storage · 2026-10-16

manager上游注册表原BTreeSet<String>的私有node与String创建未由snapshot费用
覆盖。现在使用有序KnownVec<paid_json::Text>，按真实Text数组Layout和每个
root的实际u8 Layout先准入再分配。启动时配置roots也在同一bank创建目的地，
不再先clone/collect；没有估算BTreeSet node。配置端Upstreams map/client/
credential与URL normalization仍是独立待接线边界。

插入先binary search：已有root在零额外配额和32项上限时也无需创建文本或数组。
新root先准入/创建文本，再reserve实际数组（增长时原/新backing同时收费），
最后以预留slot插入；任何拒绝均不修改原表，临时文本退款，原key和backing
指针保留。比较与JSON数组排序保持原BTreeSet行为，快照继续借用root并创建
各自独立paid文本。删除移出原Text，guard释放后销毁；若返回值继续存活则
保持原指针和费用，数组仍作为可复用已收费storage保留至registry销毁。

HTTP仍在mutation前构建成功回复；新增存储拒绝保持ResourceLimit，返回503，
不误报成功或触发Source。原32项count拒绝、重复add、URL规范化/鉴权、删除
present/absent规则不变。所有private fixture也改用同一fallible paid insert，
没有test-only未收费插入路径。测试换bank时只允许空registry并同步重绑定。

新增5项覆盖allocation hook时真实文本/数组均已付、逐一差一字节拒绝与原
key/backing指针保留、零额外配额duplicate/full/删除后重用、部分初始化退款、
移出文本离开registry后的最后owner，以及实际HTTP晚期growth拒绝/原resource
类别/表不变/重试/duplicate指针/删除/完整退款，Source/usbmux调用均0。

最终专项5PASS、完整tunnel587PASS/0FAIL/2原ignored、上游访问6PASS；
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager
1项通过；host5PASS/0FAIL/22NOT_RUN，Source/usbmux trap0，
real_device_executed=false。621个冻结源码无漂移。core/Cargo未改，保留此前
fb89530 core诊断7/8及strict证据，不冒充重跑。
CLI SHA-256：`b258c31300d654ee7f05a09054a08ac8bf5df47a9b629765ae55cbe26143f5a0`。

最终日志、源码清单、受测binary/current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-upstream-registry\round3`。
round1保留fixture误用未启用的reqwest Response::json编译失败；round2保留4项
单元PASS与fixture最终退款过早断言失败。HTTP fixture显式Connection: close，
并在1秒内等待原accepted connection owner退款后，round3全部通过，未改生产
语义或金额断言。

其余URI/header/client/credentials/policy/auth/framework/WS/private IO/error/
Arc/allocator/TaskCell、manual副本、initial公开RSD目录以及原完整freshness/
刷新/复用/重启/manual epoch/选择/prune条件继续。128MiB是已收费自有对象池，
不是完整manager heap/RSS上限。L02不关闭，23交付/6缺开发/L02待完整审计、
有效30和Q04全部排除保持。仅本地提交，无真实设备、push、发布或部署。
