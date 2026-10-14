# L02 native RX buffer 归属 · 2026-10-14

manager Registry 保存原全局 byte bank，既有 empty-only test rebind 同步更新。
Reservation/Attempt 通过新增默认 None 的 TunnelReservation::byte_bank 向
activation 传递同一个 bank；现有非 manager 构造器保持兼容。Registry 原已收费
body 的 size_of 包含新增 Arc。原 NativeLease/runtime/handle、attach/complete/
retirement、绝对期限与 native joins 不更换。

userspace 在 listener 或 worker 创建前准入并创建真实40-byte header。reader
校验 IPv6/MTU/header 后，在 payload read 前按 packet 请求容量准入，再
try_reserve_exact；实际 capacity 超出已准入量即拒绝，实际费用保持至最后 buffer
持有者。不可 Clone 的 OwnedBuffer 仅暴露 slice，原 buffer 移动经过 pending
send、8192 channel、ChannelDevice.rx_buf、SmolRxToken 和 consume closure。
Vec 在 permit 之前析构，queue/reader/device 的较早退出不提前退还后来 owner
持有的 packet 费用。

原 reader IO error 仍进入 OwnedTask/Ledger join。准入失败是 typed
BufferAdmission，ResourceLimit 分类不匹配诊断文字；TryReserveError 与整数
转换原 cause 保留。oversized/非IPv6 discard 与原 no-bank packet 语义保持。
没有 empty packet 或虚构设备 ACK，也没有借新 NativeLease 替代原 runtime。

新增10项覆盖真实 capacity/exact/一字节拒绝、原转换 cause、header 拒绝前
listener/worker/IO0、packet 拒绝只读 header、真实 blocked send、8192 queue 到
最后 RxToken 的同 buffer pointer/fee、partial native IO 的非零原 payload 地址、
oversized discard、真实 activation duplex handshake 的 bank 传递，以及 manager
原 Registry bank。真实 reader fixtures 保留严格 Ok(0) EOF、OwnedTask/Ledger、
绝对 join deadline 和 caller Drop 前后的 task baseline。

外部 overlay9文件先核对全部原/proposed raw SHA，再一次应用；冻结567个
构建源文件。userspace30、activation4、manager bank1通过；最终core tunnel101、
classification12、CLI tunnel485通过，0失败、2原ignored。core/CLI all-feature/
all-target strict Clippy、CLI build/fmt/diff、readonly manager1 Source/usbmux trap0
全部通过。host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。
CLI SHA-256：`54b5f8b76152935f1944982e5a94ec01e2c781fed54db8c144bf4062687a950f`。
raw Git blob/working source 与提交关联由 postcommit-proof.json 核对。

证据根目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-native-rx`。
外部冻结、guard、原/proposed源码和 patch 在相邻 l02-native-buffer-draft；narrow
日志在根目录，最终完整门禁/冻结/受测 binary/proof 在 final。无设备发现、恢复、
配对、安装、push、部署或发布，其它设备/平台/SDK/feature/MSRV 保持真实证据
或 NOT_RUN。

本项只覆盖真实 RX Vec capacity。8192 channel count 不覆盖 channel 私有节点
或 rx_buf backing；runtime/task/report/error、allocator rounding/overhead 也未
计入。TxToken 真实 fatal outcome、socket两个1MiB buffer、proxy scratch/chunk/
partial send、queue/socket/map backing、publication 与完整 freshness/manual epoch
继续交付。128MiB 不是完整 manager heap/RSS 上限，L02 未关闭；23交付/
6缺开发/L02待收口、有效30和Q04全部排除保持。
