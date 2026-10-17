# L02 exact prepaid selector storage · 2026-10-17

failure identity、retry row 和 Source receipt 原先先预留 body/字段长度，再用
普通 clone/to_owned，事后检查容量。本项保留原整笔预算，改为用同一原 grant
创建字段的精确实际缓冲区。metadata_vec::from_grant 在分配前验证原 bank、
Layout 与 permit 字节数一致，共用 exact_backing 的原分配器及 hook；不建
新 bank、不临时重复收费、不靠分配后的容量检查接受缓冲区。

Copier 将 body 和已创建字段的费用留在 retained，未使用字段 credit 单独
保留在 available。每段文本只从 available 拆出精确 grant，创建实际 u8 Vec，
费用移入 retained 后 String::from_utf8 复用同一指针。后续字段不能再使用
body 或先前缓冲区的费用。全部复制完成后合回原最终 owner；调用方使用早先
绑定的 storage 变量接回费用，部分失败中字段仍先析构、Copier/费用后释放。

三处生产创建端均接入：Identity::copied、FailureReceipt::reserve、
Removed::episode，包括 UID 和 identity host。route family、None/空 host、
native_id、独立原 selector owner 与共享 Identity clone 保持。receipt 的
Original/wrapper 等全部 capsule 费用仍先于任何 selector 复制和 Source poll；
原 typed IO、history/completion、episode/namespace、回滚顺序、绝对期限、
native handle/join 和严格 EOF 规则保持。最终金额、结构大小和旧预算断言未改。

新增 7 项专项通过：原 grant 在零余量下高对齐分配，实际元素先析构后退款；
异池、短/长 grant、Layout overflow 分配前拒绝退款，空 buffer 零分配；所有
route 家族的精确原 pointer/容量；已有 UID 及大量 body credit 不能支付差一
字节的后续 host；实际共享 Identity 最后 clone；实际 receipt 的 capsule
差一字节拒绝零 selector 分配、零余量复制和最后 owner；实际 retry node 原
key/host 指针、native_id 和退款。既有 manager/回滚/原错误测试完整保留。

首轮专项 6 PASS / 1 FAIL 暴露初版 helper 将已持费字段 credit 合回可用区，
后续字段可重复拆用的真实问题。修复为 available/retained 分离，并用 body97
加强原短额测试，没有放宽预期。首轮日志保留。最终专项 7 PASS，完整 tunnel
678 PASS / 0 FAIL / 2 原 ignored，upstream access 10 PASS；CLI all-features/
all-targets Clippy -D warnings、build/fmt/diff、实际只读 manager 1 项通过。
Source/usbmux trap 0，host 5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

650 个冻结源码无漂移；core/Cargo 未改，既有 core 证据复用，不声称重跑。
受测 CLI SHA-256：
`07c01c062147c3dc5aec180a94aba1f7caf01332363886015d3541981a65c266`。
最终日志、source delta、受测 binary/current 和提交后 rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-selector-copy\round2`。

本项证明实际 selector 字段的创建与最后 owner，不证明原 Box/Arc body 的
最终 allocation 寿命。其他 body 费用仍需审计（例如 Node grant 位于 Box 内），
发现/SDK 原输出、公开初始 RsdHandshake 逃逸、HTTP/client/config/framework/
private IO 与完整 freshness/刷新/复用/重启/manual epoch 继续。128MiB 为已
准入对象池，不代表完整 heap/RSS；L02 未关闭，23 交付/6 缺开发/L02 待全审计、
有效 30 和 Q04 全部排除保持。无真实设备、push 或发布。
