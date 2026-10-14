# 当前工程调度

更新：2026-10-14。原定义见[工程路线图](engineering-roadmap-2026-09-30.md)，
开发计数与真实验收分开，Q04及全部子项继续排除。

当前批次修复独立review确认的R01–R15。起点af20f6a，修复分支
codex/review-fixes-20261010，原交付基线993a366和迁移起点660bd46保留。
审查否定了Antigravity的30/30声明：承接原19项和L03，本轮A03开发交付后为
原L04测量与资源归因工具也已交付，Q02原范围开发收口后，当前23项可承接、6项仍缺开发、L02待完整核查。
见[待开发清单](development-backlog.md)和[本轮修复/验收](validation/review-fixes-2026-10-10.md)。

| 范围 | 本轮工作与验收出口 |
| --- | --- |
| Viewer/S01/S02/F05 | Principal设备/端口范围与撤销、DES认证；真实PNG/Web/VNC共享源、设备输入/文本剪贴板、统一owner预算与退出。原HEVC/方向/图像子项独立登记。 |
| CDP/F04 | 清除生产常量和空成功，真实WebInspector请求/响应/event关联；[原生context关闭/独立刷新](validation/cdp-native-context-closure-2026-10-13.md)已接线，pinned实际客户端10通过，默认Playwright初始化拒绝保持。其它Runtime语义、Page/iframe/Input/Fetch/指标各域原交付继续，F04未关闭。 |
| SDK/A03/L01 | A03开发交付：apps/AFC/logs共同checked trust owner、Python/C预算/取消/并发/close/join、公开aclose，installed wheel与C实际ABI gate分别30场景通过；[SpringBoard请求owner/总期限](validation/springboard-lifecycle-2026-10-13.md)及CLI接线已验，L01其它profiles继续。 |
| Restore/F06 | 拒绝未实现执行、失败非零与取消/terminal；实际restored传输子项、IPSW全组件CRC和TSS请求关联；完整模式转换/票据/ASR/FDR仍需代码与协议证据。 |
| Q02/F02 | 正确矩阵CLI及manifest、精确操作/失败/hash/清理；真实包ID与本轮安装owner、统一finally、log原task取消/join、全阶段失败fixture。WDA和依赖已本地提交94574d6；Windows watch原console/Job/reader工具9项通过，见[本轮证据](validation/windows-watch-2026-10-12.md)。Q02已按原FUP/矩阵交付物核查完成开发收口；[终态CI判定/最终receipt](validation/f02-terminal-integrity-2026-10-13.md)实际CLI26方法通过；现代全流程、结果准入及整体Rust组合仍缺代码/证据，真实验收仍待条件。 |
| F01 | 真实平台枚举与目标route、[目标personalization manifest/目录观察](validation/doctor-manifest-directories-2026-10-13.md)已接线，60项fixture通过；本地BuildManifest/组件/签名全证据及原平台/设备范围仍缺，未执行项保持Unknown。 |
| L02 | Snapshot/publication、coordinator、selection/plan、candidate输入/索引、[写入批次](validation/tunnel-write-batches-2026-10-13.md)及[manual desired route](validation/tunnel-manual-route-bytes-2026-10-13.md)同池owner/准入已接线，64MiB发布子限保持。[desired listing原key/backing](validation/tunnel-desired-list-bytes-2026-10-13.md)、[scan自有胶囊](validation/tunnel-scan-capsule-bytes-2026-10-13.md)及[write owner/context payload](validation/tunnel-write-payloads-2026-10-13.md)创建前预算及[native registry结构backing](validation/tunnel-native-registry-bytes-2026-10-13.md)也已接线；[retry/history自有存储](validation/tunnel-retry-owned-storage-2026-10-13.md)的真实backing、胶囊和删除owner在Source前准入，433项通过；[扫描起始publication快照](validation/tunnel-scan-publications-bytes-2026-10-13.md)实际backing/key同池准入，整组436项通过；[manual/suppressed借用索引](validation/tunnel-scan-desired-index-bytes-2026-10-13.md)真实backing收费，原436及新增2项通过；[USB唯一身份临时索引](validation/tunnel-usb-identity-index-bytes-2026-10-13.md)真实backing同池准入，整组440项通过；[raw/selected USB借用索引](validation/tunnel-usb-attachment-index-bytes-2026-10-13.md)actual backing同池准入，整组442项通过；[manual pending/planned借用容器](validation/tunnel-manual-paired-plan-bytes-2026-10-13.md)真实backing同池准入，原442及新增2项通过；[seen共享Input/paired observed完整视图](validation/tunnel-seen-observed-bytes-2026-10-13.md)真实backing同池准入，整组450及最终预算14项通过；[failure表原node/key/route及backing](validation/tunnel-failure-table-owners-2026-10-13.md)创建前准入、原位rollback与最后owner退款已接线，整组462/最终成功清理11项通过；[publication pruning原owner](validation/tunnel-publication-retain-owners-2026-10-14.md)临时backing同池准入/原key-value回滚/guard外析构已验，469项通过；[manual success Source前准入](validation/tunnel-manual-success-preflight-2026-10-14.md)完整bounded backing/canonical key/当前pending proof与authority-ticket回滚已验，481项通过；[retry selector创建前归属](validation/tunnel-retry-selector-owners-2026-10-14.md)共享原identity并先收费再复制receipt/episode，484项通过；[native RX真实buffer](validation/tunnel-native-rx-storage-2026-10-14.md)从创建前准入到最后RxToken/closure退款已接线，core101/CLI485及strict门禁通过；[当前RSD目录/原lease交换](validation/tunnel-rsd-current-owners-2026-10-14.md)checked catalogue/SDK resolver/cache-table-registry fees与原cause/期限已接线，CLI498和最终freshness16通过；[publication真实表/node](validation/tunnel-publication-table-storage-2026-10-14.md)完整actual backing/key/payload创建前双池准入、原node回滚及全部guard外析构已验，CLI504/最终46通过；[native Tx/socket/chunk](validation/tunnel-native-transmit-storage-2026-10-14.md)token前准入/原fatal、真实queue和2MiB socket/partial buffer最后owner已验，core110/CLI504及strict门禁通过；[SocketStorage真实backing](validation/tunnel-socket-storage-2026-10-14.md)动态预付growth、原Meta/handle/state和scope view已验，core114/CLI504及strict通过；[channel真实message backing](validation/tunnel-owned-channel-2026-10-14.md)fixed storage创建前准入/FIFO/cancel/close/原buffer费用已验，core121/CLI504及strict通过；原error私有payload创建端、其它manual副本/其余backing完整全局归属与刷新规则继续，[未计量边界](validation/manager-error-payload-2026-10-12.md)明确，L02未关闭。 |
| L04已交付 | 同fixture前后1/4/16与16轮启停/断连、生产native/AFC/Syslog连续资源采样、默认堆归因与基线派生性能参考范围。见[证据](validation/manager-resource-baseline-2026-10-11.md)；物理设备和其他平台NOT_RUN，超出参考范围保持观察标志。 |
| 原门禁维护 | 能力登记含目录mod.rs、Windows五项core失败诊断；保持原错误source、严格EOF、native join/worker恢复断言。 |

用户最新允许独立子agent加速；Rust/Cargo/maturin仍由主线程串行。开发检查使用冻结源码，
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
