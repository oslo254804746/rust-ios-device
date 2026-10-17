# L02 returned ConnectedDevice Box · 2026-10-17

NativeSource 的具体 Source future 已持费，但成功后另建的 ConnectedDevice Box
仍是独立分配。本项在 Source future 创建及首次 native IO 前，先用原全局池
准入并分配精确的空 `Layout<ConnectedDevice>`。成功时把原返回对象放入同一
分配并转换为 Box，没有第二次分配。独立 owner 的 Box 字段在 permit 前，
对象和原 allocation 析构完成后才退款；费用随返回对象离开 Source frame。
Source 构造拒绝、取消或原 async 错误都会销毁尚未初始化的目的地并退款。

发布的 64MiB 子限仍计算原完整 retained metadata；全局账只减去同一个 bank
已独立拥有的精确 Box 本体费用。不同 bank 不能取得抵扣，不把保守的 RSD
HashMap/字符串估计当作本次实际 allocation，也不把短 Source 费用当最后 owner。
私有未计量 connect 仅留在 cfg(test) fixture；NativeSource 的该入口明确拒绝，
生产只能通过持费的 connect_owned。公共 SDK、原 native lease/任务 join、
绝对期限、错误来源和 epoch 规则保持。

新增 6 项专项通过：高对齐原 pointer、返回 owner 跨 Source frame 的费用和
析构顺序、差一字节零分配、空目的地取消、Source frame 构造拒绝退款、原
typed IO pointer、异池抵扣拒绝及 ZST 零分配；NativeSource 旧 fixture 无 IO。
已有真实 NativeSource 四种 route 的构造测试改为验证两次精确预付分配，
零/本体差一字节/本体加 frame 差一字节均精确拒绝；始终不 poll 生产 Source。

最终完整 tunnel 654 PASS / 0 FAIL / 2 原有 ignored；upstream access 10 PASS。
CLI all-features/all-targets Clippy -D warnings、build、fmt、diff 检查通过；
实际只读 manager 1 项及 host 5 PASS / 0 FAIL / 22 NOT_RUN 通过，Source/usbmux
trap 0，real_device_executed=false。642 个冻结源码无漂移，core/Cargo 未改，
既有 core 证据复用，不声称重跑。受测 CLI SHA-256：
`793486a79f7aa3109d7155dad8674b5093edda20d8cfa21b6800e115003ce3c6`。

日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-returned-device-box\round1`。

继续刷新/发现/SDK future、Source raw host/UDID 与返回元数据创建端、初始公开
RsdHandshake 目录逃逸、config/client/URI/received/relay/framework/private IO
和完整 freshness/刷新/复用/重启/manual epoch 审计。128MiB 仍是已准入对象池，
不是完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30 和
Q04 全部排除保持。仅本地提交和隔离 fixtures，无真实设备、push 或发布。
