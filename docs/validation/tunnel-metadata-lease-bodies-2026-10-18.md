# L02 independent metadata fee payloads · 2026-10-18

原 ManagedTunnel/Registry/RefreshProof 的两个 Arc<OwnedSemaphorePermit>
payload 未单独计入 managed_metadata_bytes，且原 permit 在 allocation 结束
前析构。本项将两类私有费用根接入独立 Lease owner。完整引用审计未发现
这两类根的 Weak/pin/raw Arc 出口；每个根通过 Shared 消费原 allocation，
allocation 结束后才析构原费用和自身 body 费用。

每个实际 Lease payload 包含原费用 permit 与另一池的 body permit。原费用
包含该池对自身 body 的准入，两池均在任一 root 创建前预付两个实际 payload
的 sizeof，不另建 guard allocation 或预算池。工厂核验原 bank/整笔最小
body 费用，再从每笔原 grant 拆出另一 root 的 cross body。每个根的两池
费用持续到自身最后 owner，原 global/publication 根仍独立退款，没有因
共用一个容器而推迟另一根的原费用。

managed_metadata_bytes 增加实际两个 payload 字节；原独立 native device
body 与 route 字段的 global 扣减保持。table/registry/proof 根与原地址比较
均接入同一封闭 owner，native handle/catalogue/version/IO Arc 未改。精确
fixture 只增加实际新 payload 费用，原 SourcePeak 已销毁 Source Box、零余量、
table 与 joins 分离退款规则保留。failure slot 的计数池未改；private Arc
headers、opaque IO/error 及完整 heap/RSS 不据此计为完成。

新增4项通过：任一原 grant 的 foreign/差一字节拒绝均退两池；任一费用根
单独存活时仍保持自身在两池的 body、原费用与原指针，闭池下各自最后退款；
8线程并发最后 roots 原 bank/地址和单次退款；实际 native table 移除与
原 joins/strict EOF/task baseline 完成后，外部原费用 roots 继续独立持有，
逐根 Drop 只退该根原费用和 cross body，原可用字节最终准确恢复。

专项3、native metadata7、native freshness14、完整 tunnel741 PASS / 0 FAIL /
2原ignored、upstream access10通过。all-features/all-targets CLI Clippy
-D warnings、build/fmt/diff、实际只读 manager1通过；Source/usbmux trap0，
host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。

671个冻结源码无漂移；core/Cargo未改，既有core证据复用，不声称重跑。
CLI SHA-256：
`5b96554efb944c3c592fc243a308b3de2d763f5428fd91a4778b0f8dd680536b`。
日志、source delta、受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-metadata-lease\round1`。

failure slot 等 owner、terminal/anyhow/private header/opaque error/IO、发现/
SDK原输出、公开初始RSD、HTTP/框架任务及完整freshness继续审计。128MiB为
已准入对象池，非完整heap/RSS；L02不关闭，主项23/6/L02待全审计、有效30与
Q04全部排除保持。仅本地提交，无真实设备、push或发布。
