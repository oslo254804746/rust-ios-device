# L02 failure slot full declared allocation · 2026-10-18

原 failure count slot 的 payload 已预付并在最后 owner 析构后退款，但 std Arc
私有控制块仍未计量。本项将该真实 slot 换成应用自有的单个共享分配，声明
Allocation<T> 包含 AtomicUsize strong counter、原 T 和原 OwnedSemaphorePermit。
用此实际类型的 Layout/size 一次准入 counter、value、grant 与 padding，不猜测
std Arc 实现，不额外创建 Box/Arc/费用 sidecar。

私有 shared_allocation::Shared<T: Unpin> 使用原 metadata_vec::from_grant 的
精确 capacity-one allocator：同池与完整 layout 验证在 alloc 前完成，再将原值和
原 grant 原位放入，保留同一个指针。Clone 仅递增原 atomic count；无 raw/Weak/
mutable/pin 导出。Send/Sync 限定 T:Send+Sync+Unpin。最后 Release decrement /
Acquire fence 后只移动一次原值和 grant，用原指针的 empty len0/cap1 Vec 释放
完整原布局，再销毁原值及 grant；原值 Drop 展开时 grant 也在已释放布局之后退款。
计数溢出采用 Arc 同类的中止保护。没有改用新的 semaphore 或复制原错误。

failure_slot::Slot 仅持有原 count permit，字节 grant 位于上述真实分配内。body_bytes
现在返回整个应用自有 root 的实际 layout。reserve 保持 count 先于 bytes 的拒绝
优先级；table、episode、receipt、已退出管理器后的独立引用仍 clone 同一个 root。
原 slot 指针、count bank 身份、closed/zero pool 复用与最后一份计数/字节退款保持。

新增5项通过：256对齐 payload 的真实完整布局、首次 backing 前全额准入和少1字节
零分配；异池/错误 layout 在 backing 前拒绝并退款；真实 dealloc 之后、原析构之前
及最终 grant 退款前的次序（正常及原析构展开）；8线程各1024次 clone/drop 保持原
地址并仅一次最终析构/退款；真实 failure slot 的 whole layout 单次创建、两池先收费、
关闭原池后的最终释放。旧 slot 3项保留，包括实际管理器 shutdown/clear 后原 receipt
slot 复用和原 typed IO 指针。未用计数恢复替代 native joins。

精确共享4、failure slot4、reconnect20、error payload attribution9、完整CLI1736PASS/
0FAIL/13原ignored通过。严格CLI all-features/all-targets Clippy -D warnings、build/
fmt/diff及实际只读manager1通过；Source/usbmux trap0。host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。Core/Cargo未改，既有Core证据复用，不声称重跑Core。

首轮精确测试4项通过后，strict Clippy 提示 ptr_eq 仅测试使用，已明确 cfg(test)，
没有 allow 抑制。错误归因过滤名在执行前修正为真实 error_payload 模块；首轮因
Clippy停止，错误过滤命令未执行，不作为测试覆盖。失败日志保留。第二轮全部11项
串行验证通过，681个冻结源码无漂移，5处源码差异。CLI SHA-256：
`62bf6422ba064e0d228cac7305658e4e82a403d88610a86e392744c23432f0b8`。
日志、source delta、受测 binary、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-failure-slot-full-allocation\round2`。

本项仅关闭 failure slot 这一真实控制块边界。其它 standard Arc/Bytes/private
headers、原错误/anyhow/context、SDK/发现输出、框架task/IO/allocator及完整
freshness继续原范围审计。128MiB为已准入已知对象池，非完整manager heap/RSS；
L02未关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部排除保持。仅本地提交，
无真实设备、push或发布。
