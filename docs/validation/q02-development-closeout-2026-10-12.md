# Q02 开发收口与原 FUP 边界 · 2026-10-12

按原路线图 Q02 定义，矩阵、合成 fixture、生产操作 adapter、精确 observer、有界
捕获和 owned cleanup 均已交付。本页以 **开发完成、设备/平台验收待条件** 关闭 Q02
开发缺口；没有把 22 个 NOT_RUN 改为 PASS，也不关闭其依赖产品的 F01/F02/F05 等主项。
此前 300ad41 已提交通用 Windows capture/agent；本项补全最终范围核查发现的合同缺口。

## 本项修复与验证

- Windows agent 矩阵行计入 `host_passed`，失败或未运行不能被平台过滤漏掉。
  补齐 `owned-process-tree` 前置条件；Windows watch 与共享 Job 源码均进入证据 hash。
- 普通 adapter 单次 capture 上限从 70 秒改为原 FUP 的 60 秒，整个操作仍最多
  90 秒，明确 cleanup reserve 仍10秒。捕获异常无有效 receipt 时不能报告 cleanup 完成。
- 每步记录原 PID、cleanup scope 和 stop 状态；文件 adapter 记录实际/预期
  size/SHA256、原目标保留、长路径 UTF-16 长度及无未知输出状态，不输出私密路径。

受影响合同 **65 方法，61 PASS / 4 平台跳过**；含实际 CLI 9 个 AFC 输出场景的
size/hash/目标保留/长路径/strict peer EOF，以及应用/XCTest、observer 负合同。
最终主机 runner **5 PASS / 0 FAIL / 22 NOT_RUN**，未执行真实设备。
Rust 未变，未重编译；沿用 CLI SHA256
`b720cfce86ea2c61575480c73979f57a72c6ff8f0c89c304ea486870b049b62f`。
capture/watch 原 handle 与 reader 门禁按未变源码复用300ad41证据，不称全量 Rust/SDK重跑。

## 原交付物逐项结论

| 原范围 | 开发交付 | 继续保留的验收条件 |
| --- | --- | --- |
| 风险矩阵与生产执行 | 5 host / 22 device 行、精确 CLI/多步操作、fixture hash 与单独 scope 准入；没有剩余 UNSUPPORTED observer 占位 | 真实设备和平台运行，所有未执行行仍 NOT_RUN |
| 私有 MAC/altIRK、Display/HID、InstallCoordination、RSD 模式 | 对应生产 observer 与严格 adapter；拒绝把原始结构检查、单向 flush、RAII 或 host Job 等同设备 ACK | paired-identity CLI/mDNS、kernel Display/HID、queued/legacy/passive 实际 CLI、真实线类型和模式 |
| Windows watch、应用/文件/XCTest | Windows私有console/Job，恢复前置与finally；应用原安装owner、文件完整性、XCTest结果/日志/原native收尾 | 完整设备rich clipboard、现代完整工作流、平台/真机兼容性，不由legacy局部PASS覆盖 |
| FUP-01 | 正式排障独立说明污染输入、status8、命令级MSYS_NO_PATHCONV、严格路径区分；实际Windows Python child argv在RAM比较 | 原两份报告和证据root不可用，按原条款记录来源限制；不补造勘误。Windows/MSYS调用仍待实际环境观察 |
| FUP-02 | 默认过滤/完整目录/canonical与未知原因说明、59项仅限原轮次、未触发模式登记 | 未独立复验不可用历史原始证据，不升级成熟度 |
| FUP-03 | 当前仓库替代工具统一capture/agent/异常/清理，RAM raw与独立masked扫描，仓库外identity/key、allowlist、原进程树与精确文件收尾 | 旧仓库外工具不可用，不能声称改了它。使用所有者控制的输出目录；任意Windows ACL、其它平台和第三方外部adapter不在已验范围 |
| FUP-04 | 所有输出子例和独立UNC条件有生产adapter，普通60秒/操作90秒上限、结果与文件事实记录完整 | 所有者放置的精确AFC fixture、真实UNC共享和权限条件；NUL保持host-only。无条件不启动设备，不做三方真机对照 |

Windows ACL 是运行前必须满足的存储条件；当前工具不替用户修复任意目录 ACL。
第三方 agent/上游工具按原 FUP 规定未启动；不把缺少外部授权/设备/历史证据说成已运行。
通用仓库工具依据当前工程授权交付，不使用旧任务书“仅本文可提交”限制覆盖用户后续
明确的修复与本地提交授权。原事故与不可撤回的历史转录限制继续保留。

因此开发账本变为 **23已交付、6缺开发（L01、F01、F02、F04、F05、F06）、
L02待完整收口**，有效分母30。不是工时比例；Q04及全部子项继续排除。
下一项推进L02原错误payload与跨子系统字节归因，随后按原范围处理其它缺口。

外部证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a12eff-9622-7f01-b777-335a6d20895d\repair\q02`。
入口 `q02-summary.json` 与 `round-handoff.json`。原capture与失败制品保留。
