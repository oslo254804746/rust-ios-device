# L02 native registry/report full allocations · 2026-10-19

原 registry 和两个预付 NativeReport 的 payload 已先收费，但 std Arc 私有控制块
未计量。本项将三者改为应用声明的完整共享分配，counter/value/原 grant/padding
按真实 layout 付费；各 body fee 位于实际 allocation 内、value 外。slots backing
仍由独立原费用持有。原报告只保留原 OnceLock<TunnelError>，没有复制错误、增加
sidecar 或估算 std Arc 布局。

Registry constructor 在任何 slots/root backing 前，先取得 registry 与两个报告的
完整 grant，再取得原 slots grant；随后才创建 slots、error root、expiry root 和
registry root。NativeReport 从原 grant 的 semaphore 获取同一个 bank，用 strict
exact layout allocator，不新建池、不在 native ownership/原错误到达后申请额度。
原 first non-expiry/first expiry、当前 inline outcome、typed IO/source/priority、
冻结 snapshot、额外计数和实际 native handle/join 规则保持。最后 stock 可独立于
registry 与 native owner 存活，完整实际分配结束后再析构原值和费用。

测试夹具 rebind 移至 RegistryOwner，只在 root 独占、空 slots、无历史、两个
stock 独占且未填入原错误时允许。单次新 structural grant 包括三个完整 root 与
原 slots；在全部新费用取得并安装完成前，保留所有旧 grant。原 root/slots/
namespace 地址不变，拒绝路径不改原状态。新增测试专用 get_mut/replace_storage
具有 count1/noWeak/exclusive borrow 检查；生产没有可变导出。报告的生产去重使用
同一个实际 root 的 ptr_eq。旧 shared_value 测试 mutator 已无调用，直接删除，
未用 allow 抑制告警。

原 preflight 测试增强：0、body/stock/slot 最后一字节不足均在任何 backing 前
拒绝；精确配额观察到恰好4处实际分配（slots、error root、expiry root、registry
root），每次创建前已取得全部费用，原值地址落在正确分配范围内，不猜 offset。
保留原8线程原 IO/报告指针、独立历史费用、原池关闭后的最后退款。原 metadata
rebinding/namespace/refusal、实际用户态 native worker pending/后续完成、原 typed
cause/原 Elapsed、冻结旧 snapshot、实际 joins 与严格 EOF 等断言保持。
新增一项原语测试检查 alias 时拒绝 mut/rebind、短 grant 拒绝、独占时先取得整份
新 grant、原 root 地址与值保持、旧信用直到返回的旧 token 析构才归还，以及关闭
新池后的最终释放；换池没有替换或新建 backing。

共享分配7、retirement38、error payload9、完整CLI1741PASS/0FAIL/13原ignored、
严格CLI all-features/all-targets Clippy -D warnings、build/fmt/diff通过。实际只读
manager1通过，Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。Core/Cargo未改，Core证据复用，不声称重跑。

首轮共享分配7项通过后，strict Clippy发现旧 shared_value::get_mut 已无调用，
删除过时测试接口后重新冻结。第二轮全部10个串行门禁通过，失败日志保留。
681个冻结源码无漂移，7处源码差异；当前二进制 SHA 与实际 host regression
保存的 binary_sha256 完全一致，受测 binary 已归档：
`221c2c34796a416377164c0fc22ad0e92decdbe407bc89dcdd389268c92cbd11`。
日志、source delta、binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-roots-full-allocation\round2`。

其它 owner/marker/private headers、原 opaque Error/Anyhow/context、SDK/发现输出、
框架IO/task/allocator与完整freshness继续原范围审计。128MiB是已准入已知对象池，
非完整manager heap/RSS。L02未关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部
排除保持。仅本地提交，无真实设备、push或发布。
