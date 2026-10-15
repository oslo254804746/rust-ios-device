# L02 H2 flow-control counter storage · 2026-10-15

原bank进入pending receive credit和outbound stream window两张真实counter表。
Nonebank保持原HashMap；metered表为显式Vec<(u32,V)>，真实Layout在增长前
准入，old/new同时持费，不猜HashMap bucket布局。原key/value不复制heap对象，
clear/remove保留尚存的真实backing费用直到owner释放，无新计数截断。

receive WINDOW_UPDATE先校验两个counter的原溢出/协议错误，再一次准入本批
全部新keys，之后才能reset原counter或生成/写入window credit。拒绝时旧计数
保持、无部分reset、无window字节。open_stream在原GOAWAY/stream检查后、
原HEADERS写入前准入新outbound key；拒绝尚未IO的原connection仍可使用，
不生成空成功，不丢原typed ResourceLimit。原window更新/关闭/error范围保持。

SETTINGS先验证全部delta和range，再在同一独占owner中原位更新，不再创建
未计费updated Vec；验证与commit之间无await，失败不commit或ACK。原signed
negative windows/maxframe/delta/error规则保持。receive credit原两帧暂存改成
实际inline Option<[u8;13]>字段，去掉Vec<Vec<u8>>及两次builder heap，原wire
bytes/31bit masking/connection先于stream的顺序保持。其它outbound builder、
SETTINGS ACK/preface等未据此宣称已全部收费或无heap分配。

新增8项：真实counter old4+new8 peak末字节不足保留原values/指针；batch重复
key/preflight/replace/vacant reuse零配额与clear持费；3000步原HashMap语义对照；
真实receive新key不足时旧credit不reset也不发帧，原连接补额度后生成同一credit；
真实open新key准入拒绝在HEADERS前且保留原connection，补额度后继续；零额度
仍整批应用已验证SETTINGS并ACK（不需新scratch grant）；原invalid window错误
在零配额时保持协议优先且不commit/ACK；25组inline字节与原builder对照含
reservedbitmasking。

原pending DATA实际writer fixture现在也配置两张真实banked counter表，按实际
backing单独保留费用至framer销毁，原payload/queue/sourcefee的最后owner、
实际第5帧queue增长拒绝hook、logical rollback、strictEOF断言保持。没有为了
新收费提前改变拒绝目标或弱化buffer/pointer/原flags验证。

最终新增8项、XPC166/prepared4/existing18/core tunnel131/CLI tunnel504
均通过，0失败/2原ignored；strict core/CLI all-feature/all-target Clippy、
CLI build/fmt/diff通过。无默认feature及无tunnel raw组合(dproxy+fetchsymbols+
restore) all-target编译通过，保留实际cfg deadcode warnings，不称这两组strict。
readonly manager1，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。冻结594个build-source，当前源码无漂移；CLI SHA-256：
`fa24315a002863f4a4407d8cd238ee25dd09da1a9994efc64c8e51c560d34203`。受测binary/完整门禁/sourcefreeze和提交raw
Git blob/工作区proof均在证据目录。证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-window-storage`。

剩余H2 stream/reset maps、open/closed sets、tombstone queues、outbound encoders/
initialRSD、Bytes ownerheader、Tokio/JoinSet/waiter/Arc/error/report/allocator与
原full freshness继续。128MiB非完整manager heap/RSS；L02不关闭，23交付/
6缺开发/L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备
发现/配对/安装/恢复/push/发布/部署；未运行平台/SDK/feature/MSRV/设备保持
实际NOT_RUN。
