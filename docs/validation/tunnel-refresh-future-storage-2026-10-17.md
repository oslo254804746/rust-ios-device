# L02 original RSD refresh Source future · 2026-10-17

刷新目录的 Source 原先另建未持费的 BoxFuture，已付的 catalogue/receipt/proof
费用不覆盖该分配。现在私有 refresh_existing_rsd 返回构造 Result，使用原全局
bank 和实际具体 async F 的 Layout，在 Box 创建前准入。共用 Dynamic 直接把
同一个 Pin<Box<F>> 转为动态视图，没有先建 raw Box 再包装/补费。生产、测试
默认和 FreshSource 覆盖均使用同一构造路径；没有增加任务或公共 SDK API。

manager 在原 receipt/proof 准入之后构造 Source，构造拒绝不 poll、不记为已
执行的刷新失败、不替换原目录或 native owner。首次 poll 到 Ready(Err) 时，
仍在同一个 poll 中保存原 typed cause 到原 receipt/native/scan owner，维持
零配额、clear 和跨期限时的原错误。成功结果先保存，再显式销毁原 Source
Box 和费用，然后准入独立的输出 Metadata。原 catalogue 自有费用不提前释放。
原绝对 deadline、authority、publication/目录证明、native lease/join 与严格
EOF 断言保持。

新增 2 项专项和完整门禁通过。实际 NativeSource 直接/代理 route 构造测试
验证精确预付、原 dyn data pointer、取消退款、零/差一字节拒绝不分配，始终
不 poll 生产 Source。实际 manager 先通过 localhost H2/XPC 获取目录，再在
原 Source frame 差一字节处拒绝：refresh/建连次数保持 1，原 publication、
catalogue 和 native handle 均为同一对象，预算准确恢复；原 worker/peer 收尾。
全部既有 freshness 错误/取消/期限/交换测试不放宽。

完整 tunnel 656 PASS / 0 FAIL / 2 原有 ignored，upstream access 10 PASS；
CLI all-features/all-targets Clippy -D warnings、build/fmt/diff 通过。实际只读
manager 1 项、host 5 PASS / 0 FAIL / 22 NOT_RUN，Source/usbmux trap 0，
real_device_executed=false。642 个冻结源码无漂移；core/Cargo 未改，既有 core
证据复用，不声称重跑。受测 CLI SHA-256：
`2c1fc4f4c6b885a878d5c7ebec576b2294a82ce6982ba4d20c1926e36bb9555c`。

日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-refresh-future\round1`。

发现/SDK future、snapshot/raw host/UDID 与设备元数据创建端、初始公开
RsdHandshake 目录逃逸、config/client/URI/received/relay/framework/private IO
及完整 freshness/刷新/复用/重启/manual epoch 审计继续。128MiB 为已准入
对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、
有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
