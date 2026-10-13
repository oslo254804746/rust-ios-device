# L02 error/history 创建端缺口复核 · 2026-10-13

本次仅只读源码/锁定依赖调查，不计开发交付或完整字节准入。原error_payload采样
是下界，opaque计数、unjoined标志、固定槽位不代表字节上限。L02保持未完成。

已定位的保留点：reconnect/causes::capture_source在取得任意anyhow后才建立原Arc、
FailureReceipt、identity/key副本；reconnect::start是Source首次poll前可准备这些
owner的入口。rows/causes HashMap和retired/删除返回Vec backing仍缺准确准入。
scan::Outcomes::begin可预留自有胶囊/context，但Session::retain/Drop不得在已取得
原错误后因quota拒绝丢掉原因。core tunnel::owner::OwnedTask::join与ChildGuard
必须先durable保存原TunnelError/JoinError，再移除原handle/认证joined。

core Ledger::snapshot每次独立创建cleanup Vec/report；CLI retirement::payload_outcomes
及采样根Vec同样分配。成功ConnectedDevice的InitialRsdProbe仍可持有原CoreError，
当前Census没有遍历这一根。这些不是设备运行条件，仍属于代码/创建端合同缺口。

可实施路线是创建端先获得可兑现的byte owner，然后将实际String/PathBuf/Vec
capacity及自有capsule inline storage移入共享sidecar；first/latest、调用方报告和
历史共享原cause及owner，最后持有者释放。Source三种入口必须早于strategy/native
启动接入；native shutdown报告也需创建前合同。不能在取得原cause之后等待退款、
拒绝或仅采样再补费用，不能让budget失败冒充原joins成功。

任意dyn Error可私藏不通过source暴露的Vec/shared mutable heap；原JoinError借用API
不暴露任意panic payload，into_panic会消耗原对象。anyhow/plist/serde_json/std IO
还含私有allocation/backtrace。这些需要真实创建端或依赖allocator合同，类型名单、
size_of_val和source遍历不能证明完整大小。保持原对象的要求不允许stringify替换。

native runtime有独立未计费范围：userspace两条8192-item packet channel，最多64个
local client每个RX/TX各1MiB，以及client queues、64KiB chunks、SocketSet/task/TLS
backing。publication metadata charge未覆盖这些运行字节；若并入完整预算，应让
packet/socket/client/runtime charge随真实对象和原owner/join生命周期移动。

下一步继续可准确证明的自有容器/胶囊准入与完整刷新规则，并逐创建端闭合上述
合同；不能把外部设备NOT_RUN作为缺代码范围的免责。其它六个原主项保持未完成。
