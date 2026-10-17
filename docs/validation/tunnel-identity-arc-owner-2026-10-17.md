# L02 private Identity Arc final allocation owner · 2026-10-17

原 IdentityOwner 将 body/selector permit 放在普通 Arc value 内，最后 value
析构即退款，先于 raw Arc allocation 释放。审计确认 Identity 不暴露原 Arc、
Weak 或 pinning 接口，原 payload 是 Unpin。本项为它接入封闭 Shared<T:Unpin>，
只提供 new/clone/deref，每个克隆的 Drop 都消费原 Arc 并调用 into_inner，再
析构返回的 value。没有新 sidecar、unsafe、body/selector 预算或结构大小变化。

全部克隆均使用 into_inner 的并发合同，避免 try_unwrap().ok() 可能所有线程
都未得到 value 的竞态。封闭 API 不生成显式 Weak；标准实现先移出 T、释放
隐式 Weak/allocation，再返回 T，随后原字段和最后 permit 才析构。这一推论
仅适用于本次无 Weak 的私有 owner。依据 [Arc::into_inner 合同](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.into_inner)
及其[原实现顺序](https://doc.rust-lang.org/src/alloc/sync.rs.html#1237-1262)。

copied 与 cfg(test) moved Identity 构造器均接入，原共享 route buffer、native_id、
Deref/Borrow/Eq 和原指针保持，存活期间没有移动 value 或深复制。Option 仅用于
Drop 消费状态，存活值始终 Some；没有 Default/from-Arc/raw-Arc/Weak 逃逸或
未持费构造替代路径。T:Unpin 限制使最终移出值不破坏 pinned payload。

新增 5 项专项通过：高对齐128的原 value/host 指针跨克隆保持；最终 value
在不同位置析构时仍持原 grant，字段释放后准确退款；8线程 barrier 并发释放
只析构一次；最终析构 unwind 仍释放原字段/grant；ZST 仅在最后 clone 后析构
一次（不声称 Arc header 零分配）；实际 Identity 的8线程原 host/native_id/
pointer width 和单次退款。既有 Source receipt、精确预算、回滚、native owner
和严格 EOF 测试未调整或放宽。

最终完整 tunnel 693 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读 manager
1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。653 个冻结源码无漂移；core/Cargo 未改，既有 core
证据复用，不声称重跑。受测 CLI SHA-256：
`e054c89e7eef2f61a66728532aefafec57f77daeb77bd989249adc6df014394b`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-identity-arc\round1`。

Arc private header/controlblock 的精确大小仍未计入；其他暴露 raw Arc/Weak 的
Input/receipt/SDK catalogue 不能沿用本次无 Weak 假设。它们的最终 allocation、
发现/SDK 输出、公开初始 RsdHandshake、HTTP/client/config/framework/private IO
及完整 freshness/刷新/复用/重启/manual epoch 审计继续。128MiB 为已准入对象池，
不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30 和
Q04 全部排除保持。无真实设备、push 或发布。
