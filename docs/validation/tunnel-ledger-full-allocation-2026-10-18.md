# L02 retry Ledger full allocation · 2026-10-18

原 Ledger payload 已同池预付，sealed std Arc 在最后 owner 结束时释放原 body，
但私有控制块仍未计量。本项让 LedgerOwner 复用已验收的应用声明精确共享分配，
完整 counter/原 Ledger value/原 grant/padding layout 在唯一 backing 前准入。
Ledger value 不再内嵌 body fee；同一原 grant 位于真实 allocation 内、value 外，
整个 allocation 结束后再析构原 State 和 grant。不创建 sidecar 或猜测 std Arc 布局。

原 policy、Mutex<State>、bank、表/row/receipt、clock/backoff/admission 与 native
所有权规则保持。State::new 只建立空 KnownVec 与原 bank 的 Arc 引用，不执行 IO/
backing allocation。constructor 使用同一个原 bank，完整 body 付费后调用 strict
zero-field from_grant；没有新池或额外 budget fallback。State 后续动态 backing
仍由既有独立原费用持有，不声称静态 Ledger layout 已包含这些独立 buffer。

扩展原两项实际测试，保留全部原断言：0/少1字节拒绝现在还禁止 backing；精确
funding 的 allocator hook 观察恰好一次实际 whole layout，原 Ledger 地址位于其
范围内（不猜 offset），每次实际分配前已扣完整费用；原 grant 与 bank 身份不变。
关闭原池后，8线程共享同一原值并在最后 owner 退款一次。实际 Attempt 在外层
Ledger drop 后继续保留 body；独立 snapshot 保留原 typed IO 地址和 Pending 完成
观察，Attempt finish 后仅返回实际整个 Ledger body，最后 snapshot 再归还其独立
费用。没有为了此次低层更换新增镜像测试或改变既有重试/清理语义。

Ledger2、reconnect21、error payload9、完整CLI1740PASS/0FAIL/13原ignored、
严格CLI all-features/all-targets Clippy -D warnings、build/fmt/diff通过。
实际只读manager1通过，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。Core/Cargo未改，既有Core证据复用，不声称重跑。

全部10个串行门禁通过；681个冻结源码无漂移，2处源码差异。CLI SHA-256：
`ed2b4d65ba8b80e2ab538d4398a9ba5f4578296d1e522c24c75b6bf10bdbd615`。
日志、source delta、受测 binary、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-ledger-full-allocation\round1`。

本项仅关闭 retry Ledger 这一实际共享分配。其它 owner/marker/private headers、
原 opaque Error/Anyhow/context、SDK/发现输出、框架IO/task/allocator与完整freshness
继续原范围审计。128MiB是已准入已知对象池，非完整manager heap/RSS；L02未关闭，
23交付/6缺开发/L02待全审计、有效30与Q04全部排除保持。仅本地提交，无真实设备、
push或发布。
