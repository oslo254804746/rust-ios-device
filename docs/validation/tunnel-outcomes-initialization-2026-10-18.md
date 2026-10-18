# L02 Outcomes initialization owner · 2026-10-18

Outcomes 原长期 Arc payload 未持有自身 body grant。本项在原 global metadata
池取得实际 sizeof<Outcomes> 后才创建八槽协调器；费用包含 permit 字段。
工厂返回 Result，生产初始化传播原 ResourceLimit，不另建池或采用未收费路径。
完整引用图仅由 manager 和 Session 持有，未发现 Weak/pinning 出口；两者统一
持有封闭 OutcomesOwner，每个克隆消费原 Arc，allocation 结束后才析构 payload
及退款。此合同限定在该无 Weak 引用图，不推导其它 raw Arc 的释放顺序。

原八槽上限、Source 前子胶囊准入、一次冻结/旧 snapshot、关闭、原错误和
Session 收尾保持。测试初始化预算增加实际 body 字节后，原子项零余量及
差一字节边界不变；native/Source 精确测试采用实际结构费用，没有额外 padding。

新增 3 项通过：零额度/body 差一字节拒绝与最后克隆退款；子胶囊差一字节
拒绝、补齐及 Session 独立保留原 coordinator；snapshot 超过 coordinator
寿命仍持有原 typed IO 指针与自身 root/body 费用，最后 owner 全额退款。
scan 全组 16、native metadata 6、完整 tunnel 718 PASS / 0 FAIL / 2 原 ignored，
upstream access 10 PASS；all-features/all-targets CLI Clippy -D warnings、
build/fmt/diff、实际只读 manager 1 项通过。Source/usbmux trap 0；host
5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

662 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`5374e8283988883786d744b40de89a083ef8bae6cb155fe11500a25e6753d13c`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-outcomes-init\round1`。

其它 manager/Freshness/Publication owner、私有 header、终态/opaque error/IO、
发现/SDK 原输出、公开初始 RSD 及完整 freshness 等继续审计。128MiB 为已准入
对象池，非完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30
与 Q04 全部排除保持。仅本地提交，无真实设备、push 或发布。
