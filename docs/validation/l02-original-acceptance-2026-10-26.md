# L02 原始多设备隔离验收 · 2026-10-26

本次按用户明确选择，优先收口原路线图 L02；ESP、嵌套分片、完整 owner 审计等扩展
要求保留为欠账，后续单独处理，不作为本次 L02 的完成门槛，亦不写成已经完成。
原定义仍以 [工程路线图 §8 L02](../engineering-roadmap-2026-09-30.md#l02多设备隔离) 为准。

## 原验收逐条核对

| 要求 | 生产实现与执行证据 |
| --- | --- |
| 以设备为单位协调；同设备竞争不重复建连 | Lifecycle canonical gate + serial；same_device_requests_publish_one_owned_tunnel、scan_and_explicit_start_share_same_device_owner 同设备只调用一次 Source、只有一条发布行 |
| 限制同时建连数 | 原 MAX_CONNECTS=4 保留；global_connect_limit_and_clear_release_all_owned_futures 与 parallel_sixteen_candidates_start_four_and_restore_all_permits 检查实际 Source 峰值及清理后全部许可恢复 |
| 全局锁不包围网络等待；A 超时不拖慢 B | connect_and_store/refresh 在网络等待前释放表锁；http_stalled_a_does_not_delay_b_list_get_stop 对 B 三次实际 HTTP 的合计耗时断言 <1s，同时 A 仍 Pending；reconnect_native_auto_a_total_expiry_keeps_b_http_query_and_stop_isolated 用真实原生 owner 检查 B 查询、停止各 <1s，A 原1.5s总期限独立耗尽 |
| 拔插/停止/刷新竞态不污染新行 | stop_cancels_active_and_queued_generation_then_allows_new_start、stop_during_discovery_rejects_unpublished_snapshot、backoff_new_native_id、freshness_old_paired_browse_cannot_retire_concurrent_replacement_publication；旧 ticket/原行身份与发布事务保持 |
| 关闭/查询不留悬挂记录 | shutdown_admission 与 native_retirement 用例验证原 runtime/joins、严格 EOF、表与注册表清理及许可退款；失败的原生清理保留可见 retirement owner，不误报完成 |
| 扫描退避继续生效 | scan_backoff_and_disconnect_release_remain_observable 验证首败→退避跳过→时间到后重试成功→拔除后移行；changed_paired_host/new_native_id 不继承旧身份的错误退避 |

上述用例均使用隔离主机 fixture。通过的是明确断言和实际结果，不依据锁结构推断性能。
原 L04 的真机吞吐/延迟不在本次 L02 验收内；其他平台与设备运行证据不冒充 PASS。

## 本轮最后的具体修复

- Unix 同一个清理 job 的 deadline 取已授权 deadline 的最大值，较短的并发 waiter 不再
  覆盖仍有效的长 deadline；各 waiter 仍由自己的原始外层 timer 限时。
- native cleanup 的完成凭证与“收齐包括失败在内的 joins”分开。新增只读
  has_pending_native_cleanup，CLI retirement 只有在原 joins 已收齐且没有 pending native
  cleanup 时释放 owner；Windows 的 body-completed 标志移到实际撤销流程末尾。
- Registry 移出已完成 entry 后再析构 metadata/费用，避免 semaphore waiter 在锁内被唤醒。
- 补真实 blocking-job 时钟回归、实际 JoinError/保留凭证回归，以及已经变更地址、原调用
  超时后才返回同一个 receipt 的 LocalOps 回归。迟到结果先收齐真实 join，再按新显式清理
  时钟撤销；零剩余配额和严格 EOF 检查保留。

没有增加通用框架、备用 arena、任意资源上限或完整进程堆证明。测试新增仅针对本次
具体缺陷，稳定后执行一次完整 CLI 回归，不以每个小改动反复全量验证。

## 结果

最终源码共780个 Cargo/crates 输入，全部与冻结清单匹配，零漂移。实际命令、原日志和
退出码保存在仓库外证据目录（下述路径）；每条原验收用例的实际 PASS 行另存
`original-acceptance-proof.json`，不把相似名字或搜索命中当作通过。

| 检查 | 实际结果 |
| --- | --- |
| 完整 CLI（含14个已核对的原验收/本轮修复关键用例） | **1927 PASS / 0 FAIL / 13 原 ignored**，用例执行262.51秒 |
| Linux Unix actor | **11/0**，含本轮3个新增回归；用例执行3.54秒 |
| Windows Core tunnel | **245/0** |
| metadata退款锁外唤醒 | **1/0** |
| Windows Core/CLI、Linux Core strict Clippy | all-features/all-targets `-D warnings` 通过 |
| 实际 CLI build、fmt、diff | 通过 |
| 只读 manager / host | **1/0**（trap=0）、**5/0/22 NOT_RUN**；real_device_executed=false |

Windows环境：Windows NT10.0.26100、rustc1.90.0；Linux为WSL Ubuntu-22.04。
没有进行设备或系统网卡操作。780个输入的同一清单覆盖green2的Windows严格门禁与verify
门禁；首次验证中断后，确认Windows与WSL无残留构建进程才恢复，未并发构建。

证据目录：

`C:\Users\wangbaofeng\.codex\visualizations\2026\10\25\01a1736a-ccc6-78d0-9bae-82b6c174a8f8\l02-unix-continuation-2026-10-26`

实际当前二进制、归档 verified-ios.exe 与 host report provenance SHA-256 一致：

`ec49d074360032cf734ea0cbdf899c9f13f73b57737b4c1a17b08b91ad739ec5`

开发红灯也保留：原时钟实现0/1失败；添加实际panic机制后9/1失败。修复后11/0。
迟到地址测试脚本化的是native操作/receipt边界，不冒充真实netlink ACK解析或设备验收。
Darwin本轮未重跑真实系统验证，不新增macOS运行结论。

## 明确保留的扩展欠账

1. ESP、未知 native transformation、内层普通分片的完整私有流隔离。
2. 任意实际 netlink/pending ACK 响应增长，以及已暴露 native cleanup body 真正 panic
   后的自动恢复。当前已修复错误完成/过早释放，未声称能自动恢复任意 panic。
3. 原要求之外继续扩展的逐类 owner/opaque SDK 物理分配审计与完整模式证据矩阵。
   已有128MiB只表示已知对象池；未宣称完整heap/RSS上限。
4. 真实 iOS、macOS 实机与系统网卡/地址/路由配置运行证据，仍 NOT_RUN。

欠账保留于本文，不新增或偷换原主任务分母，也不算交付。按用户认可的原范围收口后，
有效30项为24项开发交付、6项仍缺开发：L01/F01/F02/F04/F05/F06；Q04全部排除。
