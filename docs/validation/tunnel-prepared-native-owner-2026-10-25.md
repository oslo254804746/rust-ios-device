# L02 私有准备与同一 owner 的原生激活 · 2026-10-25

此前原流已有唯一 PacketPort、四个原始 worker 和 attach-before-launch。
本次新增在同一认证流上完成私有 RSD 检查的 Core 准备阶段，以及 Windows
同一 handle 的原生激活、实际 join 与撤销。旧连接入口和 ConnectOptions
字段保留；CLI 仍使用原 connect_owned 路径，本次尚未接通 CLI 的完整 staged
publication/replacement，也不关闭 L02。

## 实现

四个 additive prepare_*_owned_with_owner_at 入口复用原认证策略和绝对时钟，
要求原 byte bank。CDTunnel 参数交换后，原四个已准入 future 先附到同一个
Arc，再启动。可选初始 RSD 与必须成功的独立 checked catalogue 各自走真实
私有 IPv6/TCP/H2/XPC；后者验证原 UDID，原初始目录不被刷新覆盖。

PreparedConnectedDevice 保留原截止时间、设备与目录 owner。最终 userspace
开放同一 listener；Windows Kernel 使用 finish_kernel_at。两者均把请求时钟
夹到原 preparation 截止时间，返回同一内部设备、handle 和 checked owner。
大的准备 future 在构造和 Box 分配前支付其实际类型费用，避免默认栈溢出。

Kernel 先关闭并 join 私有 listener、clients 和 smoltcp，将同一 PacketPort
归还；原 transport reader/writer 不重建。原 writer 的 FIFO flush barrier
完成后才交给原生桥，部分原始 read/write 仍由原 worker 持有。

Windows KernelRuntime 在原 handle 的一次性目的地内持有实际 control、work、
bridge、undo 和 launch 根。原生 init/read/write/undo 的完整 blocking 调用、
起动 gate、稳定 join 槽、MTU 接收 backing、单包 pipe、filter 和独立 FlowLease
均先准入；bridge 的实际 future 在 NativeCreate 前准入。原 adapter 分配结果
先存入原 owner，再解析身份、设置 MTU/session/address/route。配置写锁在开放
packet gate 前释放，DAD 和最终 readback 使用共享借用。

Ready 来自原身份、完整 /64、MTU、实际 Preferred 地址和 explicit/derived
route 读回，并等待 init 的真实 join。最后一次 DAD 读回仍为 Tentative 时继续
原时钟，冲突保留实际 dad_state；本地 Ready 查询核对同一持有资源及原凭据。
Windows 只绑定 Create/Close，不打开或接管旧 adapter，不删除共享 driver。
[Wintun 头文件](https://git.zx2c4.com/wintun/tree/api/wintun.h?id=544fdaa)
区分创建与打开资源的关闭行为；本实现仅采用创建路径。

跨原四个任务、native 和 undo 的三个真实 Ledger 共享一次首因记录点，保持
原错误 Arc、深层 IO、JoinError 与原 Elapsed。报告按观察到的原槽掩码和计数
保持不可变，不复制大数组、不在失败时新增 Box/费用、不跨 Ledger 比较局部
sequence。原十槽、原四个任务和原计数不复用。重复输入的析构在锁外。

停止先设置状态、封闭 pipe、触发真实事件，再通知/abort 原传输。桥持续监听
原 reader/writer 的真实 join，背压不能掩盖原 EOF。已 stopped 的状态仍重试
失败的同一停止事件；首个 IO 保留在原 undo 槽，重复失败计入既有计数。
收集器先收齐 native 和原任务/clients，再执行预付费 undo 并 join，最后释放
原 port、registry 和 native FlowLease。取消的等待者不能移走资源或 join。

私有连接使用真实、排他的 IPv6 wildcard 端口保留；完成流的精确 tuple 和
描述符跨过最终原生任务。fragment filter 使用原包 owner 和已付费动态数组，
覆盖 offset zero、out-of-order、重叠、atomic 与首次到达起算的协议生命周期。
IPv6 地址投影、opaque transformation 等语义审计仍未完成，不能据此宣称
任意扩展头下的完整隔离。

Linux 与 macOS 同时加入 ordinary-owned 原生 facade 和纯 wire/row codec。
Linux 保留 ACK 后即时 MTU receipt、未消费的实际 response/pending operation，
不把 response backing 拒绝改写成清理成功。macOS 新建 unit0 utun，取得真实
FD 后立即返回 owner，后续身份/down 操作原位进行；route backing 上限来自
协议 u16 长度。两个平台的最终 Kernel actor 尚未接线。

## 验证

全部 Cargo、fmt、build、主机测试由主控串行执行。最终 round2 对冻结的
771 份源码逐项复核，无修改或新增；Core/CLI all-features/all-targets
Clippy -D warnings、fmt、diff 检查通过。实际执行：

- Core tunnel 231、device activation 16，全部通过。
- 完整 CLI 1903 通过、0 失败、13 原有 ignored；未新增忽略。
- Kernel-only 与 userspace-only check 通过，分别仍有45和90项 unused/dead-code
  warning，未把这些最小组合称作严格无警告。
- Windows 同 owner 的完整脚本原生路径覆盖双向原包、explicit/derived route、
  DAD 取消/到期、真实阻塞 identity/undo 调用及关闭重试、独立撤销失败、首次
  native IO Arc/深层指针、严格 peer Ok(0) 和已计费对象退款。
- 真实两次私有 RSD 后，经同一 SDK 完成方法返回同一 Arc/catalogue/字符串
  指针；仅原生函数表注入，不运行默认设备构造 future。
- 背压与停止信号两个新用例先在旧生产正文下失败，均在失败断言前恢复实际
  joins/资源。修复后四项 actor 用例通过；原生原因被取消掩盖的另一回归也先
  取得7通过/1失败，再修复为8通过。
- WSL 对同一 checkout 编译 Linux Core/native 模块；刚生成的同一 ELF 实际
  执行16项纯 codec/row 测试通过，没有调用 LinuxDevice 创建或接口操作。
- 外部最小 Cargo harness 引用原 native/mod.rs、固定同一 libc0.2.182；
  x86_64-apple-darwin 与 aarch64-apple-darwin 的 check --tests 通过，包含原
  macOS facade、测试和实际 libc ABI/offset 断言。没有链接或运行 Darwin 程序，
  也没有把组件检查称作整个 Core 的 macOS 验收。
- 当前构建的实际 CLI 只读 manager 用例1通过，host5通过/0失败/22NOT_RUN，
  real_device_executed=false；Source/usbmux trap 保持0。

最终实际与归档 ios.exe 的 SHA256 均为
`dab21f23e39d3fc1f68fb07910218081e1bbb56fe24fa5b370bd786332f51a44`。
完整命令、源码清单、日志、状态与二进制位于仓库外的
`l02-prepared-kernel/round2`；失败开发轮次也保留。主控复核为本次有限证据，
不冒充独立的 L02 最终审查。

## 仍需完成

CLI 的准备结果、NEW record/global/publication/fresh metadata/native attachment
及精确旧 owner proof 必须在原生暴露前一并就绪，随后接通原 publication、
rollback、自然结束旧 runtime、NEW 返回值与原 retry/backoff/window/authority。
Linux/macOS actor、IPv6 语义隔离和全 retainer/opaque SDK 边界审计继续。
128MiB 指已准入已知对象，不是完整 SDK heap、驱动内存或 RSS 证明。
没有进行真实设备、网卡、地址或路由操作。23项开发交付、6项仍缺开发与
L02待全审计口径不变，有效30项，Q04及全部子项继续排除。
