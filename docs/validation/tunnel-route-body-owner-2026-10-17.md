# L02 retained route body final owner · 2026-10-17

retained route 原 body permit 位于 raw Arc value 内，已有 Weak 观察却不能
证明 backing 释放后才退款。本项统一接入 RouteOwner=shared_body::Strong<
RetainedRoute>，覆盖 Desired::Manual、ManagedTunnel、Cell/lifecycle 的 manual
共享接线及扫描借用索引。数据引用先结束、body guard 后结束；text 字段在
最后 strong 时正常释放，Weak 继续保留 body allocation 费用，最后 Weak 才退款。
[Weak 的 value/backing 合同](https://doc.rust-lang.org/std/sync/struct.Weak.html)
继续适用；原 weak 不保活 text 或 native 资源。

创建前按实际 RetainedRoute 与新增 guard 已知 payload 取得 body grant，
同 bank/精确金额校验后才创建数据与 guard Arc。text 使用原精确目的地费用，
值内保留同池零字节 permit 供原 bank 校验；cfg(test) moved-input 构造器则
保留原 String 指针/capacity，其字段费用与 body 分离。没有新 bank、未付
sidecar、重复 body 收费或私有 header 尺寸猜测。预算 helper 和 backing 使用
实际 RouteOwner/body 类型，原数量/字节子限不变。

原 family、None/空 host、Mobdev2 desired filter 与 observed discovery host
的区别保持。manual live shortcut、同 runtime 恢复、authority/epoch、外部
expiry 回滚和 Source 前拒绝路径均未改变。原 route 地址比较改为配对 owner
地址比较，不深复制 host，不触及其他 gate/version Arc。

新增 3 项专项通过：实际 Cell manual filter/原 host pointer 跨 snapshot 和
Stopped 替换保持，最后 strong 释放 text 后 body 仍由 Weak 持有；8线程最后
克隆保持原 host 指针并精确退款；零/body 差一字节/host 差一字节拒绝零 text
分配、部分退款和同池零 permit。专项合计4项（包含原 body/text 边界测试）。
真实 native 元数据 fixture 加强为两阶段精确断言：表移除后 route value 已
结束但 body 尚持费，显式释放原 Weak 后再核对原 body 退款金额。原句柄、
Input owner、shutdown、实际任务 join、严格 EOF 和旧配额断言保持。

最终完整 tunnel 707 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；
all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、实际只读 manager
1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。658 个冻结源码无漂移；core/Cargo 未改，既有 core
证据复用，不声称重跑。受测 CLI SHA-256：
`3c0ee62c3b43e5fdff7ea33435d7fb74de5fbe7097dbf4e381cc23a19fa8f0d6`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-route-body\round1`。

其他 Box/框架共享 owner、私有 Arc header 精确字节、opaque error/IO、发现/
SDK 原输出、公开初始 RsdHandshake 和完整 freshness/刷新/复用/重启/manual epoch
审计继续。128MiB 为已准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/
6 缺开发/L02 待全审计、有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
