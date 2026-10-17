# L02 original discovery Source futures · 2026-10-17

USB/paired discovery 的原 BoxFuture 尚未持费。生产私有 Source 现在要求
usb_devices_owned/paired_devices_owned，接收原全局 bank，在实际具体 async F
的原 Box 创建前准入；Dynamic 直接 unsize 同一个 allocation，析构后才退款。
没有先分配再补费、额外包装 Box、任务或 deadline。旧入口和 fixture 默认
adapter 仅 cfg(test)，NativeSource 旧测试入口明确拒绝设备 IO；其内部 fixture
Box、原 SDK future 和返回 snapshot 创建端仍是独立未收口边界。

四处生产调用均接入：手动默认 paired fallback、设备建连后的 USB attachment
复核，以及扫描 USB/paired discovery。原 frame 在独立 snapshot 准入前析构。
DiscoveryAdmission 区分构造拒绝与真实 async IO 错误，设备已连接后的 USB
构造拒绝不会再触发 paired fallback。扫描 paired 构造拒绝保留原 ResourceLimit
并形成不完整视图，已经预付的 USB 候选、failure destination 和实际执行 window
仍能运行；它不能用不完整视图删除旧 paired publication/backoff。原 async
错误、绝对期限、clear/authority/epoch、readiness 和 native 收尾规则保持。

新增 6 项专项通过：实际生产 USB/paired 构造器精确预付、原 data pointer、
零/差一字节拒绝无分配，始终不 poll NativeSource；旧测试入口无 usbmux/mDNS；
实际 manager 的 USB 拒绝保留旧 publication/backoff、paired 拒绝后 USB 仍发布
且未知 backoff 不变；实际 native 建连后 USB 拒绝 Source1/USB0/paired0，不
fallback，原句柄 shutdown、严格 EOF 和原任务基线恢复。

首轮专项 6 PASS，完整 659 PASS / 3 FAIL / 2 原 ignored：三个精确预算 fixture
没有为新增的临时发现 frame 留费用，提前拒绝，未到原 pending/index 边界。
最终 fixture 使用各自真实构造器在独立测量池创建但不 poll，测量并完整退款。
pending paired/USB 添加精确 frame 费用，维持原 available=0 断言，取消后精确
增加该原 allocation 退款；index 测试在 paired 原 frame 已析构后占用同一临时
增量，恢复原差一字节拒绝和旧金额。没有硬编码猜测 Layout、生产延迟退款或
放宽原 native/published 金额、句柄、join/EOF 断言。首轮日志保留。

最终全局预算 17、native 元数据 6 项通过；完整 tunnel 662 PASS / 0 FAIL /
2 原 ignored，upstream access 10 PASS。CLI all-features/all-targets Clippy
-D warnings、build/fmt/diff、实际只读 manager 1 项通过；Source/usbmux trap 0，
host 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。645 个冻结源码
无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。受测 CLI SHA-256：
`aec675232c206f9f8cc361b68a165735c4f938f7d945388b78f126aa46e5a0d2`。

最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-discovery-future\round2`。

原 Source/candidate 文本与返回 discovery snapshot/设备元数据创建端、SDK future、
初始公开 RsdHandshake 目录逃逸、config/client/URI/received/relay/framework/
private IO 和完整 freshness/刷新/复用/重启/manual epoch 审计继续。128MiB
为已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全
审计、有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
