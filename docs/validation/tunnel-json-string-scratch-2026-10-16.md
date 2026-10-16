# L02 upstream JSON string scratch · 2026-10-16

paid_json原destination准入不覆盖serde_json内部escaped-string Vec。本次继续
使用Cargo.lock的serde_json 1.0.149结构/数字/深度规则，通过其公开SliceRead/
Read::parse_str将实际scratch接到同一bank；没有修改依赖或复制JSON语法实现。

仅含反斜线的输入先按bytes.len()+1实际Layout创建已收费语法副本，将字符串
内容逐字节改成ASCII placeholder，保留引号、所有非字符串字节和原字节位置。
未闭合字符串补一个synthetic引号，使visitor能返回原reader的EOF/escape错误；
补充字节不进入输出。结构parser逐token请求visitor，placeholder原指针offset
定位原body。原SliceRead从完整原输入单调推进，故全局line/column保持；只在
原parser请求token时解码，较早的结构/number/depth错误仍先返回。

escaped token的原始内容字节数是解码上界：按该容量创建实际已收费u8 Vec，再
交给锁定reader的extend/push，未转义字符串借用原slice、scratch容量0。复制
最终Text时syntax/scratch/destination同时持费，原scratch Vec先销毁再退款。
语法副本及scratch离开parse后全部退款；最终树/文本独立持费并可离开原body。
原serde_json::Error直接保存在Context，保持typed cause及消息/line/column，
ResourceLimit仍返回原cause，不被中间serde custom error覆盖。

新增4项：三处实际Layout及peak allocation hook、syntax/scratch/destination
逐一差一字节拒绝与精确退款；固定无效输入逐一比较原error类别/行列/Display和
早期错误次序；全部JSON escape、Unicode/代理对/重复key/混合plain token；
实际localhost listing/target的转义字段、扩展、原全局JSON错误和Source/usbmux0。
既有独立文本离开body、深度与语法测试保留。均为固定回归fixture，Q04仍排除。

JSON专项10PASS、生产remote JSON3PASS；完整tunnel582PASS/0FAIL/2原ignored，
上游访问6PASS；all-feature/all-target CLI Clippy -D warnings、build/fmt/diff
及实际只读manager1通过；host5PASS/0FAIL/22NOT_RUN，Source/usbmux trap0，
real_device_executed=false。618个冻结源码无漂移。core/Cargo未改，复用此前
fb89530 core诊断7/8及strict证据，不冒充重跑。
CLI SHA-256：`a6085dc2ba30fbe39756757114c783d2077f5615685c53113998d6dccf217ca3`。

最终日志/冻结清单/受测binary/current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-decoder-scratch\round3`。
round1为JSON专项通过；round2保留新增fixture误用未prepare Listing造成的E0277，
修正为真实prepare流程和同一registered root后，round3全组通过。

取舍是转义输入额外持有一份已收费body-sized语法副本及单个token scratch；
没有CPU/RSS性能声明。URI/header/client/上游registry/credentials/policy/auth/
framework/WS/private IO/error/Arc/allocator/TaskCell、initial公开RSD目录和原
完整freshness/刷新/复用/重启/manual epoch/选择/prune仍继续。128MiB仅为已
收费自有对象池，不是完整manager heap/RSS；L02不关闭。23交付/6缺开发/
L02待完整审计、有效30和Q04全部排除保持，仅本地提交，无真实设备或发布。
