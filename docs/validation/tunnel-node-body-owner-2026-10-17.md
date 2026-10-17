# L02 failure Node body final owner · 2026-10-17

原 Node 的 body permit 位于 Box 内，字段析构先退款，不能证明原 Box allocation
已经释放。本项将 Node 改为 OwnedNode=owned_box::Owner<Node>，实际空目的地
在 identity/key/Source 前精确准入并分配，同一原指针填入 Node 后进入表、移除
批次和回滚。body permit 位于 Box 外，Box 在前、permit 在后，原 allocation
释放完成后才归还该 body 费用。

全部持有路径已接入：next、Table/Destination/KnownVec、RemovedBatch、Candidate、
DeferredFailure、prepared 参数及返回值。backing/minimum/header 预算使用实际
OwnedNode 宽度；初始 USB window/destination 仍在 paired discovery 前预付，
不能用旧 Box 指针宽度代替新 wrapper。原 key permit 继续独立跟随 String，
success intent 复用原 key/grant；body 和 key 的寿命不互相代替。

成功路径调用 Owner::into_inner：原 Box 交出指针，同布局单元素 Vec 接管，pop
将 T 移到栈上，显式销毁空 Vec 原 backing 后才退款，再将原字段移入 success。
没有依赖 `*box` 表达式何时释放 allocation，没有再分配或复制 key。该转换
保持原 global allocator、T/Array<T>(1) 的同 size/alignment、一个初始化元素和
唯一 ownership；ZST 使用原 aligned dangling pointer，无分配或双重析构。
这些条件依据 [Box::into_raw](https://doc.rust-lang.org/std/boxed/struct.Box.html#method.into_raw)
和 [Vec::from_raw_parts](https://doc.rust-lang.org/std/vec/struct.Vec.html#method.from_raw_parts)
的原始指针/布局合同；不使用 nightly Box::into_inner，也不改变原 pin future。

新增 5 项专项通过：实际 Node 原 pointer/body grant 跨 table removal 到最后
owner；body 差一字节零分配，不消耗原 key；identity 拒绝时真实空 body 目的地
准确退款且 key 未取用；generic high-align128 取值不新分配，body 退款后原值才
单次析构；ZST 无分配、单次移动和析构。旧 outcome key hook 仅允许新增的
精确 Node Layout body 分配，仍禁止额外 key buffer；success key/grant 测试
保留，原表/回滚/配额/native owner/EOF 断言没有放宽。

最终完整 tunnel 683 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；
CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、实际只读 manager
1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。650 个冻结源码无漂移；core/Cargo 未改，既有 core
证据复用，不声称重跑。受测 CLI SHA-256：
`68bd41b08054a31f40872b23ddf3c3b518f46e2e1c82a178b3fa0b78d72e54df`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-node-body\round1`。

其他 body 最后 owner（例如 reconnect Removed、共享 Arc payload/weak/control
blocks）、发现/SDK 输出、公开初始 RsdHandshake、HTTP/client/config/framework/
private IO 与完整 freshness/刷新/复用/重启/manual epoch 审计继续。128MiB 为
已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、
有效 30 和 Q04 全部排除保持。仅本地提交，无真实设备、push 或发布。
