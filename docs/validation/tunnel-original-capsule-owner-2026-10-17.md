# L02 private Original capsule final owner · 2026-10-17

receipt 私有 Original capsule 将 permit 放在 Arc value 内。审计确认该原
capsule 不暴露 raw Arc/Weak，本项将 FailureReceipt.original 和 History.first
接入已验证的 sealed Shared<Original>：每个克隆消费原 Arc、into_inner 返回
原 value 后才析构错误字段和 permit。存活期间原错误不移动、不复制或格式化，
first/latest 继续共享各自原 capsule，构造/预算/结构大小均不变。依据
[Arc::into_inner 合同](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.into_inner)
及[原实现释放顺序](https://doc.rust-lang.org/src/alloc/sync.rs.html#1237-1262)，
此无 Weak 私有路径在 capsule allocation 结束后才归还已准入 payload 费用。

没有新增 sidecar、unsafe、SDK API、时钟或原错误包装。全部 capsule 仍先于
selector 复制和 Source poll 准入；原 Ready(Err) 同 poll 捕获、first/latest
选择、history/completion、clear/abandoned 和 native joins/EOF 规则保持。
其他 raw Arc receipt 或 opaque anyhow/IO allocation 不由此自动证明。

新增 2 项专项通过：实际 Original 的8线程最后克隆保留同一 typed IO pointer/
kind，原 IO payload 析构时 grant 仍持有，随后单次退款；实际 History.first
和 latest 的独立原 IO pointer、原 grant 分别在各自最后 root 结束后归还。
pointer width 与已知 capsule 大小保持，原精确预算和错误断言未调整或放宽。

最终专项 2 PASS，完整 tunnel 701 PASS / 0 FAIL / 2 原 ignored，upstream
access 10 PASS；all-features/all-targets CLI Clippy -D warnings、build/fmt/diff、
实际只读 manager 1 项通过。Source/usbmux trap 0，host 5 PASS / 0 FAIL /
22 NOT_RUN，real_device_executed=false。656 个冻结源码无漂移；core/Cargo 未改，
既有 core 证据复用，不声称重跑。受测 CLI SHA-256：
`0894047849c084aafa1bd4fcd8ec87ac6c8b645a817885f927cc1899488fb857`。
日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-original-capsule\round1`。

FailureReceipt/其他 Arc body、Weak 和私有 header 精确字节，发现/SDK 原输出、
公开初始 RsdHandshake、HTTP/client/config/framework/private IO，以及完整
freshness/刷新/复用/重启/manual epoch 审计继续。128MiB 为已准入对象池，不代表
完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、有效 30 和 Q04
全部排除保持。仅本地提交，无真实设备、push 或发布。
