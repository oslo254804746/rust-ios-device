# 当前工程调度

更新：2026-10-12。原定义见[工程路线图](engineering-roadmap-2026-09-30.md)，
开发计数与真实验收分开，Q04及全部子项继续排除。

当前批次修复独立review确认的R01–R15。起点af20f6a，修复分支
codex/review-fixes-20261010，原交付基线993a366和迁移起点660bd46保留。
审查否定了Antigravity的30/30声明：承接原19项和L03，本轮A03开发交付后为
原L04测量与资源归因工具也已交付，Q02原范围开发收口后，当前23项可承接、6项仍缺开发、L02待完整核查。
见[待开发清单](development-backlog.md)和[本轮修复/验收](validation/review-fixes-2026-10-10.md)。

| 范围 | 本轮工作与验收出口 |
| --- | --- |
| Viewer/S01/S02/F05 | Principal设备/端口范围与撤销、DES认证；真实PNG/Web/VNC共享源、设备输入/文本剪贴板、统一owner预算与退出。原HEVC/方向/图像子项独立登记。 |
| CDP/F04 | 清除生产常量和空成功，真实WebInspector请求/响应/event关联；实际pinned客户端验证每域，未知字段/无法保证语义明确拒绝。 |
| SDK/A03/L01 | A03开发交付：apps/AFC/logs共同checked trust owner、Python/C预算/取消/并发/close/join、公开aclose，installed wheel与C实际ABI gate分别30场景通过；L01其它profiles继续。 |
| Restore/F06 | 拒绝未实现执行、失败非零与取消/terminal；实际restored传输子项、IPSW全组件CRC和TSS请求关联；完整模式转换/票据/ASR/FDR仍需代码与协议证据。 |
| Q02/F02 | 正确矩阵CLI及manifest、精确操作/失败/hash/清理；真实包ID与本轮安装owner、统一finally、log原task取消/join、全阶段失败fixture。WDA和依赖已本地提交94574d6；Windows watch原console/Job/reader工具9项通过，见[本轮证据](validation/windows-watch-2026-10-12.md)。Q02已按原FUP/矩阵交付物核查完成开发收口；现代全流程与真实验收仍待条件。 |
| F01 | 真实平台枚举与目标route，未执行remoted/DDI可用性为Unknown；不凭文件名或socket猜定Ok。 |
| L02 | [Raw snapshot/publication](validation/tunnel-global-known-metadata-2026-10-12.md)、[coordinator](validation/tunnel-coordinator-bytes-2026-10-12.md)、[USB selection](validation/tunnel-selection-bytes-2026-10-12.md)、[paired plan](validation/tunnel-paired-plan-bytes-2026-10-12.md)、[candidate索引](validation/tunnel-candidate-index-bytes-2026-10-12.md)及[candidate输入/backing最后owner](validation/tunnel-candidate-owners-2026-10-12.md)同池准入已接线；64MiB发布子限保持。原error/history创建端、outcome collectors/manual route/其余backing完整全局归属与刷新规则继续，[未计量边界](validation/manager-error-payload-2026-10-12.md)明确，主项仍未关闭。 |
| L04已交付 | 同fixture前后1/4/16与16轮启停/断连、生产native/AFC/Syslog连续资源采样、默认堆归因与基线派生性能参考范围。见[证据](validation/manager-resource-baseline-2026-10-11.md)；物理设备和其他平台NOT_RUN，超出参考范围保持观察标志。 |
| 原门禁维护 | 能力登记含目录mod.rs、Windows五项core失败诊断；保持原错误source、严格EOF、native join/worker恢复断言。 |

当前按用户最新要求不派发子agent；Rust/Cargo/maturin串行。开发检查使用冻结源码，
集中验收固定源码hash与受测提交。不会终止其它会话进程或覆盖用户改动。
修复可本地提交，不推送、发布、部署。
用户于2026-10-12明确要求：每完成一个可验收工作项，验证后立即本地提交，
写明改动、验证和剩余边界，再继续下一项；不把提交等同于原主项全部关闭。
已依次提交94574d6（协议/WDA组合）、ff70516（Windows watch owner）。
随后[通用Windows capture/agent接线](validation/windows-capture-2026-10-12.md)已完成；
[Q02完整交付核查](validation/q02-development-closeout-2026-10-12.md)及报告/预算修复已完成，
Q02计开发完成但真实设备行仍NOT_RUN；下一项推进L02全局原error payload归因。

用户后续统一安排真机。没有指定设备操作范围，本轮不执行恢复、擦除、配对、
安装等设备变更；真实设备流程仍NOT_RUN。POSIX pthread、Linux/glibc资源探针、
macOS/Linux及Python最低版本/其它制品平台按真实运行范围保留NOT_RUN。
原19项是交付账本，不冒充本次全部平台重新验收。缺代码或协议的路径不能只列
硬件NOT_RUN或靠--force打开。独立可完成的修复和实现继续推进。
