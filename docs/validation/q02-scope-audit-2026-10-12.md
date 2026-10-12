# Q02原FUP交付范围收尾核查 · 2026-10-12

后续更新：本页以下保留 db5487f 时的范围核查。现通用 Windows capture/agent 的
原 Job/process/reader 接线已实现并验证，见[通用捕获交付](windows-capture-2026-10-12.md)。
不能再把“仅回收 leader”作为当前缺口。后续全部原交付物及预算/报告合同已核查，
Q02计开发完成，历史来源/存储/设备运行边界保持，见[最终开发收口](q02-development-closeout-2026-10-12.md)。

此项承接原路线图Q02对FUP-01–FUP-04的要求；不是新增主任务，不改变有效分母30。
94574d6提交协议/WDA依赖组合，ff70516提交Windows watch工具，随后继续本核查。
完整日志和精确blob/二进制/工具hash位于外部repair/watch制品目录；clean commit
门禁58方法：54通过/4平台跳过。不新增真机操作或把NOT_RUN登记为PASS。

| 原子项 | 本次核查结果 | 未关闭的准确边界 |
| --- | --- | --- |
| FUP-01 | 正式排障已涵盖污染输入、status8、命令级MSYS_NO_PATHCONV=1、`/`/`.`/不存在路径及非泛化边界；追加历史来源限制 | 两份索引的handoff报告和Windows原证据root均不可用，不能追加不存在的报告勘误或证明旧原始argv；Windows/MSYS实际argv仍需独立条件 |
| FUP-02 | 正式排障和旧任务书保留默认prefix/完整目录/canonical解析、59项限定原设备/轮次、未知缺服务原因及未触发模式 | 不独立认证不可用的旧原始证据；新Core模式/actual direct CLI不能转换为旧设备模式PASS |
| FUP-03 | 普通capture/agent原始RAM扫描、独立masked计数、仓库外identity/key、文件allowlist及原owned POSIX group已有实现；新增Windows watch private console/Job/reader已验 | `diagnostics.capture` Windows普通/agent路径仍使用leader terminate/kill；未将原Job/process-tree owner接入该通用路径。host-agent仍限定Linux；Windows ACL和第三方外部adapter边界独立保留 |
| FUP-04 | 每个Windows output子例和单独UNC条件已有矩阵/生产adapter；prior localhost实际AFC/file CLI验证9个读/输出场景，源未变 | 该9场景不是物理设备PASS，真实UNC/权限/所有者放置的fixture及原设备范围继续NOT_RUN |

本次只检查已索引位置的存在性，结果为false；未搜索或打印其他用户临时目录、私钥、
历史原始输出。正式排障历史结论仍注明来自原任务书摘要，没有伪造历史更正或数据。
旧仓库外工具未找到，不能声称已修改它；当前仓库工具按现有工程授权交付。

Q02的代表性设备行均已具备对应生产CLI/observer/fixture admission与操作scope，
但通用Windows捕获生命周期仍有上述代码/接线缺口，因此本次不关闭Q02主项。
当前仍22项已交付、7项缺开发、L02待完整收口；不是工时比例。Windows watch专用
成功不能覆盖通用capture的失败/agent路径，也不能代替设备watch通知/图像/恢复验收。

下一步：为通用Windows capture/agent接入原process tree owner，保留原stdout/stderr
字节、deadline、secret扫描与原handle/reader收尾；不能把controller metadata混进业务
输出，不能靠按进程名终止。完成该可验收子项后本地提交并继续；L02原不透明error
payload与跨子系统全局字节归因随后按原范围推进。暂不派发子agent；Rust/Cargo/
maturin串行，未变门禁按影响复用。Q04全部排除，baseline/overlay/失败制品/uv缓存保留。
