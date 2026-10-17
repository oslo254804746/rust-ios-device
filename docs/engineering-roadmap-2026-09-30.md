# 项目工程回顾与后续实施计划

日期：2026-09-30；代码基线：`572c297`。

当前批次、下一批交付边界与外部缺口见[当前工程调度](roadmap-current.md)。

状态：第一、二、三类的当前 Linux 主机范围已交付；B01/B02、Q01、S04 完成，
其余平台、部署、真机及 fixture 子项保留待条件/待验收。第四类继续 L01–L04；Q04 已按用户要求排除。按2026-10-09用户修订的开发交付口径，有效30项：19项开发完成、11项仍缺开发交付；只等待验证测试或外部运行条件不再保留为开发未完成。实际测试状态和能力证据独立保留，详见[待开发清单](development-backlog.md)。

## 1. 目标与执行范围

本计划承接本次工程回顾中已认可的建议，把项目的后续工作组织成可独立交付、
可验证、可追踪的任务。总体目标是形成可靠的跨平台 iOS 自动化与诊断基础设施，
使功能覆盖、接口兼容、运行稳定性和验证证据同步增长。

计划编写阶段的交付是本文；用户已于 2026-09-30 授权完整实施与本地阶段提交。
实施按依赖持续推进可执行任务；缺少设备、协议证据
或平台条件的任务记录具体缺口，同时继续不依赖这些条件的工作。

2026-10-06 用户明确修订执行范围：排除 Q04「协议 fuzz 与高成本专项检查」及其
全部子项，包括当前 NSKeyedArchiver UID 展开专项。原31项任务表及历史记录保留，
有效工程目标为除 Q04 外的30项及其全部子任务；排除不等于完成或验收通过。
既有 Q04 提交/证据保留，未提交实验改动已归档并撤回；不得把该专项改名后继续。
其他任务仍按原验收执行，排除 Q04 不自动解除 F06 等任务的安全或证据条件。

2026-10-09 用户修订完成口径：开发交付已完成、仅等待验证测试的任务计为完成。
本表的完成状态据此表示开发交付；原验收条款、历史结果与未执行测试仍保留，
NOT_RUN不改写为PASS，也不推导平台/真机支持或发布授权。D01/D02已交付的
文档与治理基础关闭开发状态，后续维护随新任务执行。明确尚缺代码/工具交付的
11项见[当前待开发清单](development-backlog.md)。

2026-10-08 用户要求集中修改相关代码，完成一个工作项后再统一运行回归和最终
验收；不再为每个小修复或纯文档更新重复完整测试。开发期仅在需要时做编译检查，
失败修正后重跑受影响检查，工作项结束时保留必需门禁。本批已将扫描错误保留、
全局取消和完整扫描总期限集中交付；L01/L02 的其他原验收条件仍有效。

保留四个 crate 的工作区结构和 Tokio 异步架构：

- `ios-core`：协议、传输、服务客户端和可复用工作流。
- `ios-cli`：命令解析、输出、交互及服务入口。
- `ios-ffi`：C ABI、资源所有权和错误映射。
- `ios-py`：Python API、资源生命周期及异步接入。

结构调整优先在 crate 内建立明确的模块边界；新增 crate 需有独立的依赖、
发布或使用边界依据。常规修复与重构分开交付，保持已发布接口的迁移可追踪。
参考 checkout 保持只读，比较协议行为并记录参考版本，不复制参考项目源码。

## 2. 回顾基线与证据

基线包含 102 个提交、4 个 crate、232 个 Rust 文件、约 12.5 万行 Rust 代码
（含测试），以及 62 个顶层 CLI 命令。数字描述规模，不作为质量或覆盖率指标。

下表来自本次工程回顾的 Linux 本机检查，不代表其他主机系统或真机验证结果。
本次计划编写没有重复运行这些产品检查。

| 检查 | 基线结果 | 含义 |
| --- | --- | --- |
| `cargo fmt --all -- --check` | 通过 | 格式符合仓库约定 |
| workspace 全目标、全 feature 编译 | 通过 | 主线构建可用，仍可能掩盖单 feature 依赖遗漏 |
| workspace 全目标、全 feature 严格 Clippy | 通过 | 主线通过 lint 门禁 |
| Rust 1.80 workspace 编译，排除 `ios-py` | 通过 | 该范围的 MSRV 有本机证据 |
| workspace 全 feature 测试，排除 `ios-py` | 1545 通过，0 失败，1 忽略 | 忽略项为既有高成本 PBKDF2 测试 |
| Python host 测试，关闭 `extension-module` | 6 通过 | 使用已有 `.venv` 与对应 libpython 环境 |
| 单独启用 `fileservice` | 编译失败 | 使用 `XpcClient`，未声明 `tunnel` 依赖 |
| 单独启用 `deviceinfo` | 编译失败 | 使用 `XpcClient`，未声明 `tunnel` 依赖 |
| 默认 feature 文档测试 | 5 通过，1 失败 | 示例导入了受 feature 限制的配对 API |
| `cargo test --workspace --all-features` | Python 测试链接失败 | 启用了打包专用 `extension-module`，应使用拆分测试 |

另外确认：发布任务没有依赖全部现有的 MSRV、feature 子集和 Python 冒烟检查；
部分 README 模块路径与公开 API 不一致；plist 到 JSON 的转换存在重复及编码差异。
隧道管理器没有认证，远端上游使用 HTTP/WS，部分状态变更通过 GET 暴露。

多设备阻塞风险来自源码分析：管理器持有全局生命周期锁串行连接设备，列表、
停止与手动建连也使用该锁。其具体延迟和资源影响尚未压测，后续必须补测。

协议与真机边界参考：

- [架构与模块职责](architecture.md)、[feature 说明](features.md)。
- [已有功能差异审计](reference-gap-audit-2026-09-27.md)。
- [已有功能实施记录](implementation-plan-2026-09-27.md)。
- [CDP 兼容状态](cdp-compatibility.md)、[Cryptex DDI](cryptex-ddi.md)。
- [恢复预检](restore-preflight.md)、[Wi-Fi 发现](wifi-discovery.md)。
- [真机回归后续任务](real-device-followup-tasks-2026-08-30.md)。

旧实施文档保留历史记录。本计划负责后续工程任务的调度，功能的当前事实以
维护后的能力清单和专题文档为准；出现差异时核对当前代码和证据后更新。

## 3. 优先级、阶段和任务状态

优先级表示实施顺序：P1 处理已复现接入问题、发布可靠性与访问边界；P2 建立
维护、验证和运行稳定性基础；P3 完成依赖这些基础的功能工作流。
优先级不是漏洞严重程度评级。

任务状态统一使用：`待开始`、`进行中`、`待条件`、`待验收`、`完成`。
当前开发调度按2026-10-09用户修订口径标记开发完成；技术验收仍须满足原条件。
开发交付和实际测试分别登记，已交付但缺平台/真机验证的任务不再计入待开发数量，
其未执行验证仍保持NOT_RUN或待验收。

| 阶段 | 主要任务 | 阶段出口 |
| --- | --- | --- |
| 第一阶段：接入与发布 | B01–B04、D01 的相关文档整改 | 最小接入可靠；验证入口一致；发布等待必需检查 |
| 第二阶段：证据与安全 | Q01–Q03、S01–S04 | 能力证据可追溯；管理面有访问控制；真机回归可复现 |
| 第三阶段：结构与生命周期 | R01–R04、A01–A02、L01–L04、Q04 | 模块边界清晰；输出兼容；慢设备隔离；资源释放有证据 |
| 第四阶段：用户工作流 | A03、F01–F06 | SDK、诊断、自动化和协议扩展逐项形成已验证流程 |
| 持续治理与发布准备 | D01–D02，贯穿上述阶段 | 当前文档、任务状态、兼容承诺和质量指标一致 |

阶段规定主要顺序，不要求等待某阶段全部真机条件就绪才开展独立主机任务。
具体前置依赖见下表；协议证据与设备条件属于额外验收条件。

| ID | 任务 | 优先级 | 前置依赖 | 状态 |
| --- | --- | --- | --- | --- |
| B01 | 修复 feature 依赖并验证最小组合 | P1 | 无 | 完成 |
| B02 | 修复默认文档测试与公开接入示例 | P1 | 无 | 完成 |
| B03 | 统一本地与 CI 验证入口 | P1 | B01、B02 | 完成（开发口径）：统一入口已交付；其他平台运行验证保留 |
| B04 | 补齐发布门禁与制品验证 | P1 | B03 | 完成（开发口径）：发布门禁/制品工具已交付；平台验证、registry版本/顺序与发布条件保留 |
| Q01 | 建立能力成熟度与验证矩阵 | P2 | 无 | 完成 |
| Q02 | 建立回归工具与代表性真机流程 | P2 | Q01、S04 | 开发完成：矩阵/生产adapter/observer、Windows通用capture及watch、FUP预算与证据合同已交付；22个设备行与其它平台验收仍NOT_RUN，见2026-10-12开发收口 |
| Q03 | 建立跨平台路径与输出失败合同 | P2 | B03 | 完成（开发口径）：路径/输出合同实现已交付；其他平台/UNC与实际设备验证保留 |
| Q04 | 增加协议 fuzz 与高成本专项检查 | P2 | B03 | 已排除：用户于2026-10-06明确移出当前目标；全部子项不再推进，不计完成。既有PBKDF2提交/证据保留，未提交NSKeyedArchiver实验已归档并撤回 |
| S01 | 管理器认证与设备、端口授权 | P1 | 无 | 完成（开发口径）：当前认证、授权、撤销与凭据接入已交付；平台ACL/真机运行验证保留 |
| S02 | HTTP 状态变更及浏览器访问边界 | P1 | S01 | 完成（开发口径）：既有入口方法/Host/Origin/CORS与票据实现已交付；F05新入口随F05开发，平台/真机验证保留 |
| S03 | 远端受保护传输与联邦凭据隔离 | P1 | S01、S02 | 完成（开发口径）：HTTPS/WSS与联邦凭据隔离实现已交付；部署及平台/真实设备运行验证保留 |
| S04 | 统一敏感字段和日志脱敏 | P1 | 无 | 完成：自有输出边界及 CLI 依赖日志来源已验收；新增接口继续沿用 |
| R01 | 拆分备份协议、路径与会话职责 | P2 | B03、Q03 | 完成（主机边界与回归验收） |
| R02 | 拆分 WebInspector 命令与服务职责 | P2 | B03 | 完成（主机边界验收） |
| R03 | 收敛 device 模块的文本包含结构 | P2 | B03 | 完成 |
| R04 | 统一 plist/JSON 转换与输出策略 | P2 | A01 | 完成（主机编码与stdout合同验收） |
| A01 | 建立 Rust、CLI、C、Python 兼容规则 | P2 | B03 | 完成：候选合同、版本/迁移与现有 public 字段/enum 风险登记；Linux 代表性下游/CLI/C 与 Python 3.9/3.12/3.13 通过，设备成熟度不因此升级 |
| A02 | 建立稳定错误分类与输出合同 | P2 | A01、R04 | 完成（开发口径）：当前错误分类/机器输出及binding映射已交付；legacy兼容边界和平台/真机验证保留 |
| A03 | 扩展常用任务的 Python/C SDK | P3 | A01、A02 | 进行中：`655733c` 完成 AFC caller-limit 与有总期限/累计回复预算的 Browse，`f077674` 完成 Syslog 取消续读/terminal 原生前置主机验收；`2b906d7` 完成共同 owner/checked existing-trust 原生前置；`6f582c9` 完成有限 fresh device query/预算/TCP-Unix-TLS 主机子项；Python/C 句柄、绑定/示例及真机原验收尚未完成。见 [证据](validation/bounded-service-read-2026-10-08.md) |
| L01 | 统一期限、取消和连接失效规则 | P2 | B03、R03 | 进行中：既有协议/CLI/session/native ownership 主机 profiles 已交付；本批完成 tunnel 全扫描 checked30s total、全局取消、paired/List 原错误跨 caller Drop 保留、8-slot 历史与退出报告，15s discovery/candidate 子期限保留；global retained metadata bytes、generic HTTP/upgrades/blocking/startup、physical freshness/reuse/restart/manual epoch、其余协议/清理及平台/真机原验收条件继续；`2b906d7` 完成原生 SDK owner 单一绝对期限/取消/准入子项。证据见 [扫描控制](validation/tunnel-scan-control-2026-10-08.md) |
| L02 | 隔离多设备生命周期与连接并发 | P2 | L01、S01 | 进行中：查询/停止隔离、同设备协调、4 个连接许可、snapshot admission、4096 failure/native retained owners 与 native joins 已交付；本批完成 whole-scan total/cancel、reconciliation/readiness waits、whole-clock publication/failure rollback 与 bounded discovery-error history，stop A 后 B 继续，whole expiry 不新增 backoff/ready；global metadata/bytes、physical freshness/reuse/restart/manual epoch、HTTP/upgrades/blocking/startup、CPU/平台/真机及其他原验收条件继续 |
| L03 | 增加隧道、剪贴板自动重连 | P2 | L01、Q01 | 进行中：`1a604af` 完成 serve 可配置退避、持续预算/准确状态与原 owner/epoch 回收的主机子项验收；`8947fc1` 完成 manual 实际 route/原 grant 恢复、跨扫描 stop、过期发布回滚的主机子项；Darwin watch 基线/去重/断线语义及平台/真机验收继续。见 [证据](validation/manager-reconnect-2026-10-08.md) |
| L04 | 建立性能、资源与长时间运行基线 | P2 | L02、L03 | 进行中：同 fixture 修改前后 1/4/16、30s/104轮及独立60s资源归因主机子项已验收；文件/吞吐/长时间流/平台真机未完成 |
| F01 | 扩展 doctor 的设备能力诊断 | P3 | Q01、L01、S04 | 进行中：显式设备 existing-trust/Developer Mode/DDI signature 主机子项已验收，host schema1 保留、target schema2；`838c4ed` 完成显式既有 RSD route/完整目录/目标核对主机子项，未选择 route 时仍 not_probed；平台 USB/remoted/目录、DDI usability 与真机验收未完成。见 [RSD 证据](validation/doctor-rsd-syslog-2026-10-09.md) |
| F02 | 完成应用、测试、日志与结果工作流 | P3 | F01、A02、L03 | 待开始 |
| F03 | 建立经过 Rust 桥的真实 CDP 客户端测试 | P3 | R02、S02、B03 | 已完成：`3e697fd` 固定实际客户端经生产 Rust bridge/合成 WebInspector，Puppeteer 有限 Runtime 全流程、Playwright 初始化明确拒绝、完整 trace/原 owner 回收及下载协议依据/实现方案满足原 F03 验收；F04 各域和真机验收继续。见 [证据](validation/cdp-client-workflow-2026-10-09.md) |
| F04 | 按协议域完成 CDP 兼容流程 | P3 | F03、Q02 | 待开始 |
| F05 | 完成媒体查看器、VNC 与双向剪贴板 | P3 | S02、S03、L03、Q02 | 待开始 |
| F06 | 完成恢复预检入口与实验恢复执行流程 | P3 | A02、L01、Q02、Q04 | 待开始 |
| D01 | 维护文档、变更记录与交付规则 | P2 | 首批整改依赖 B01、B02、Q01；随后持续执行 | 完成（开发口径）：文档整改与交付规则已交付；持续维护随各开发任务执行 |
| D02 | 建立质量指标与 1.0 发布准备标准 | P2 | 指标先随 Q01 建立；发布准备依赖 B04、A01、A02、L04、F02 | 完成（开发口径）：质量回顾模板、指标与1.0准备标准已交付；候选验证及实际发布准备记录保留 |

## 4. 构建、接入与发布任务

### B01：feature 依赖与最小组合

范围：`crates/ios-core/Cargo.toml`、受 feature 约束的导出及 CI feature 检查。

交付物：修复 `fileservice`、`deviceinfo` 对 `tunnel` 的依赖；自动列举公开
服务 feature、分组及别名的最小构建方式，检查每种组合；记录裁剪构建的告警来源。

验收：上述两个独立 feature 编译通过；默认、各服务、各分组与别名均通过编译。
常用组合运行相关测试。打包专用 feature 使用独立验证路径；不使用一次全 feature
构建替代最小组合检查。新添 feature 的依赖遗漏能够在 CI 中暴露。

### B02：默认文档测试与示例

范围：crate 级 Rust 文档、四个 crate README、根目录中英文 README 和入门文档。

交付物：为受 feature 限制的示例标明配置，并正确处理条件编译；修正私有模块路径；
为公开入口提供可构建的最小示例，明确 Python 与 C 当前的能力范围。

验收：默认 feature 文档测试通过；已有 `compile_fail` 示例仍验证真实的封装边界。
Rust 示例在其声明的配置下编译，说明中的导入路径与当前 API 一致。
Python/C 示例符合其资源创建、释放与错误处理规则。

### B03：统一验证入口

范围：`scripts/`、CI、[构建说明](build.md)、[Python 构建说明](python-binding.md)、
[贡献指南](../CONTRIBUTING.md) 和 PR 模板。

交付物：可在支持的主机系统执行的验证入口，支持相关模块快速验证和发布前完整
验证；CI 与开发说明使用相同的检查集合。Python host 测试与 wheel 打包分开。

验收：入口遇到失败返回非零退出码，记录具体检查项，不静默跳过必需项；说明
Python 解释器和 libpython 的选择方式，不提交本机绝对路径。正确实现跨平台调用，
不把仅在 Bash 可运行的脚本写成未经验证的通用入口。

### B04：发布门禁与制品

范围：`.github/workflows/ci.yml`、发布脚本、C 头文件与 Python 打包配置。

交付物：所有发布分支依赖必需的格式、Clippy、测试、MSRV、feature 和绑定检查；
CLI、C 库、头文件、Python wheel/sdist 的制品检查；验证与发布步骤明确分开。

验收：通过工作流依赖审查及安全的验证运行确认，必需检查失败会阻止相应发布。
发布不使用 `|| true` 掩盖必需制品缺失。C 调用示例验证签名、布局、错误码和释放
合同；Python wheel 在对应平台安装并导入，在声明支持的最低及代表性较新 Python
版本验证公开 API。检查 checksum、版本和制品清单一致。

crates.io 本地 dry-run 依赖尚未发布的同版本 `ios-core` 时，按现有打包说明记录
解析限制，安排可验证的发布顺序，不将依赖尚未入索引误报为产品缺陷。

## 5. 能力证据与测试任务

### Q01：能力成熟度与验证矩阵

范围：新增长期维护的能力清单与证据索引，连接功能、命令、平台和专题文档。

交付物：一处权威清单，分别记录实现范围与验证结果，至少包括功能/API、协议路径、
主机系统、设备/iOS、USB/Wi-Fi、必要状态、测试等级、受测提交、日期与证据位置。
成熟度分为实验、候选稳定、稳定；证据分为主机单元/协议、模拟流程、真机流程。
单项运行结果使用 PASS、FAIL、INCONCLUSIVE、NOT_RUN，不把“未知”写成通过。

验收：当前所有顶层命令和服务族能定位到记录；CDP、Cryptex、恢复、媒体及剪贴板
按当前真实边界登记。稳定声明须指向代表性真机证据与已定义兼容范围，模拟测试
不能自动升级为真机验证。缺失的服务和未知设备条件可独立表达。

### Q02：回归工具与代表性真机流程

范围：合成 fixture、假服务、真机运行工具、脱敏证据格式和回归说明。

交付物：按风险选择代表组合的回归矩阵；统一捕获实际参数、退出码、耗时、结果
及清理状态的工具；可重复运行的合成数据用例。承接既有 FUP-01–FUP-04，先检查
已完成内容，关闭有证据的项，并保留尚未满足条件的 Windows 文件输出用例。

真机验收至少分别登记：传统服务与 CoreDevice；USB 与 Wi-Fi；现代 RSD 身份和
macOS remoted 共存；私有 MAC/altIRK；Cryptex 安装；Display/HID 停止与恢复；
富内容剪贴板监听；应用、文件及 XCTest 工作流。平台矩阵按现有设备和高风险差异
选取，不要求无依据地穷举所有组合。

验收：用例指定合成数据和精确操作范围，记录前置条件、受测二进制和代码提交。
无服务、无权限或无 fixture 时如实登记原因；假服务和实际客户端流程可以在无设备
环境执行。敏感原始输出在脱敏前不打印、不归档，失败与收尾分支同样遵守。
仅清理本轮创建的文件、凭据和进程，真实证据保持用户可访问的安全存储边界。

### Q03：跨平台路径与输出失败合同

范围：AFC/备份/trace/JUnit/截图等宿主输出、原子替换和路径解析。

交付物：共享的路径与失败合同说明，以及 Linux、macOS、Windows 的代表性回归。
覆盖 macOS 系统目录别名、Windows Unicode/长路径/verbatim 路径、既有目标、
只读目标、目录目标、非法路径和临时文件收尾；明确 MSYS 参数转换的排障方法。

验收：失败保留旧目标内容，成功输出校验大小与 hash，取消后清理临时文件。
区分设备路径与宿主路径；真实 UNC 条件不足时单列 NOT_RUN。
沿用 [安全策略](../SECURITY.md) 中备份根的信任边界，不把现有路径检查描述成
能抵抗并发目录替换的原子保证。

### Q04：协议 fuzz 与高成本检查（已排除）

以下定义保留作历史范围。2026-10-06 用户明确排除本任务及全部子项；当前30项
工程目标不包含本节，不继续 UID 展开、fuzz、合成种子或高成本专项开发/运行。

范围：XPC、DTX、OPACK、NSKeyedArchiver 等外部输入解析器及关键状态机。

交付物：优先为上述解析器建立 fuzz 入口和完全合成的种子库，覆盖截断帧、错误
长度、嵌套、UID 间接引用和压缩边界；新增缺陷沉淀为最小回归 fixture。
为既有高成本 PBKDF2 测试建立独立定期或显式运行入口。

验收：预算内的 fuzz 运行不出现 panic、越界、失控分配或无限循环，记录运行
预算和种子版本；通过不能被解释为不存在未知缺陷。高成本测试有独立通过证据，
普通测试清楚说明忽略原因。保持受测代码的封装边界，不为测试随意扩张公共 API。

## 6. 管理面安全任务

### S01：认证与授权

范围：隧道 manager、设备 TCP/WebSocket 桥、联邦管理及相关 CLI 客户端。

交付物：认证配置、最小设备与端口授权策略、明确的本地与远端访问模式，以及
客户端使用说明。凭据按现有秘密文件机制持久化；非 loopback 暴露不能默认无认证。

验收：未认证和无权限的列表、连接及管理请求按合同拒绝；合法访问成功。
覆盖失效凭据、越权设备、越权端口和重新配置。默认保持本地绑定。
现有 userspace TCP proxy 的访问边界也须审查并记录；不得只保护新 WebSocket
入口却宣称所有设备转发入口均受保护。

### S02：HTTP 方法、Host 与 Origin

范围：manager、CDP、Selenium 以及 F05 后续新增的浏览器/WebSocket 入口。

交付物：Host/Origin 策略和浏览器握手认证方案；状态变更采用适当 HTTP 方法。
为旧 GET 管理入口制定有期限的迁移方案，同步更新 CLI 与文档。
原生 CDP 客户端没有 Origin 的情形与浏览器请求分别明确合同。

验收：未经允许的浏览器来源和 Host 不能建立管理/调试连接；纯读取 GET 请求
不会触发关闭或建连等状态变更。必要兼容入口遵循同样的认证与访问控制，禁止
以兼容为由留下默认绕过。浏览器不能依赖自定义 WebSocket Authorization 头的
假设；可采用受控短期会话/握手票据，长期秘密不放入可记录的 URL。

### S03：远端传输与联邦凭据

范围：上游 URL、HTTP 请求、WebSocket 连接和 manager 间目标选择。

交付物：HTTPS/WSS 的受保护接入，或明确的受保护主机传输路径；证书校验、
每个上游的独立凭据及配置说明；迁移现有仅 HTTP/WS 的远端合同。

验收：证书或凭据错误明确失败，默认不关闭证书校验；凭据只发送给其配置的
上游，不随重定向或联邦列表转发给其他主机。跨跳选择、EOF、关闭、背压、
hop 上限和循环检测继续通过。需要保留明文的本地/受保护转发方式时明确限制。
继续把 `/connect` 和 hop 标记为本项目协议扩展。

### S04：敏感字段与日志脱敏

范围：tracing、CLI 错误、HTTP/CDP 回复、Python/C 错误、Debug 实现及回归工具。

交付物：设备标识、配对记录、私钥、认证材料和设备数据的统一展示策略；默认
日志使用非原始标识。显式面向用户的设备选择输出与诊断日志分别规定用途。

验收：以合成秘密检查 stdout、stderr、Debug、异常、超时和清理路径；完整 UDID
不直接进入默认诊断日志。错误仍包含定位所需的阶段、类别和关联信息。
测试区分原始泄漏检测与脱敏副本结果，不能以副本无命中证明原始日志安全。

## 7. 模块结构与公共契约任务

### R01：备份模块

范围：`services/backup2/` 及 CLI 备份调用。

交付物：分离协议编码/解码、路径与宿主文件操作、过滤/manifest、会话执行和清理；
每层使用明确内部接口，保留公开调用和既有安全边界。

验收：备份、恢复、过滤、manifest、取消和输出失败回归通过；重构前后的合成协议
交互和结果一致。同步文件操作继续使用适当的阻塞线程边界，资源预算不退化。

### R02：WebInspector 模块

范围：CLI `webinspector.rs` 与 core WebInspector 服务。

交付物：分离命令入口、HTTP/WebSocket 适配、CDP/Selenium 会话和可复用工作流。
继续让 core 持有协议状态和服务行为，让 CLI 调用这些接口。

验收：现有 Target/session/Runtime、请求路由、错误和清理回归通过；参数、端点
与输出的兼容变化有记录；F03 可以注入合成 WebInspector 服务并启动真实 Rust 桥。

### R03：device 模块边界

范围：`device/mod.rs` 及其通过 `include!` 引入的连接、发现与辅助实现。

交付物：转换为有明确可见性与输入/输出的普通子模块；区分设备发现、连接策略、
lockdown、隧道、RSD 与服务路由的职责。

验收：USB、TCP lockdown、direct RSD、userspace 和 kernel 路径的既有主机回归
通过；公共路径保持兼容，最小 feature 检查通过，不用文件移动替代依赖边界整理。

### R04：plist/JSON 与输出策略

范围：core、CLI、FFI 中重复的 plist/JSON 转换及渲染逻辑。

交付物：先登记现有 Data、Date、Uid、整数、非有限浮点和未知值的编码，再抽取
共享转换函数；通过显式策略支持历史上使用数字数组或十六进制的命令。

验收：现有公开命令、C 输出的 fixture 不变；新接口使用已记录的策略。
不能为去重静默改写历史 JSON。秘密脱敏与普通转换独立表达，输出失败明确返回。

### A01：兼容性规则

范围：Rust 类型与导出、CLI 参数和 JSON、C ABI、Python API、版本与迁移说明。

交付物：标出稳定候选与实验接口，建立添加字段/枚举分支/方法/命令时的兼容规则；
为新类型合理使用构造器、builder、私有字段和可扩展设计；登记版本变更策略。

验收：使用代表性下游 Rust 调用、CLI fixture、C 调用和 Python 导入检查兼容性。
对已发布类型添加私有字段或 `non_exhaustive` 本身可能破坏兼容，必须先评估并
提供迁移说明。feature 重命名保留必要别名，公共路径变化能够追溯。

### A02：错误与机器输出合同

范围：`CoreError`、各服务错误、CLI、HTTP、Python/C 映射和结构化输出。

交付物：可编程判断的超时、取消、断开、服务缺失、权限、协议与输入错误分类；
明确 stdout/stderr、退出码、流式 JSON 和部分失败的含义，保留原始错误链。
在现有错误类型上按兼容规则演进。

验收：调用方可以依据稳定字段/类型决定处理方式，不依赖匹配自然语言字符串。
JSON 命令无意外混入诊断文本；错误保持脱敏。既有输出或退出码变更有迁移路径。
区分“服务不存在”“元数据缺失”“尚未探测”与“显式不支持”。

### A03：常用任务 SDK

范围：`ios-py`、`ios-ffi`、core 已有的应用/文件/日志和设备查询接口。

交付物：第一批围绕设备信息、应用列表、文件查询/有界读取、日志流扩展 Python/C
入口；继续保留现有隧道桥。提供 Python 上下文管理与异步接入方式、C 句柄和
释放接口，以及可运行的最小示例。

验收：有界输出、关闭、重复关闭、取消、错误分类、并发限制和主机绑定测试通过；
C 头文件与实际 ABI 匹配。长时间调用遵循 Python GIL/异步及 C runtime 合同。
逐项记录使用的 feature 与真机证据，不以 SDK 导出存在宣称设备功能已验证。

## 8. 生命周期与运行稳定性任务

### L01：期限、取消与连接失效

范围：连接建立、服务请求、HTTP2 流控等待、长时间流和清理动作。

交付物：区分建连、单次请求、总操作、空闲与清理期限的统一约定；请求发送和
接收共享总预算；明确取消/超时后是否能复用连接，清理失败保留主操作错误。
以已有 InstallCoordination 的契约作为一种可参考实现，按协议逐项适配。

验收：覆盖发送阻塞、迟到回复、分帧接收、调用方取消、零预算和清理超时。
失败连接不能错误消费旧回复；无限期监听采用可取消的空闲等待，不能误用短请求
期限导致正常监听退出。保持每条流的缓冲、队列与待回复状态有界。

### L02：多设备隔离

范围：隧道 manager 的生命周期锁、刷新、建连、列表、停止和失败退避。

交付物：以设备为单位协调生命周期，限制同时建连数量；避免持有全局锁执行网络
等待；处理同设备重复建连、拔插和停止之间的竞态。

验收：使用假设备让 A 持续超时，B 的本地查询和停止仍在预设测试预算内完成；
先以 1 秒作为主机测试目标，记录执行环境，验证耗时不随 A 的建连期限累加。
同设备竞争不生成重复隧道；关闭与查询不会产生悬挂记录；扫描退避继续生效。
真实吞吐与延迟交由 L04 测量，不能仅据锁结构声明性能达标。

### L03：自动重连

范围：隧道/RSD 连接与 `pasteboard watch`，以及依赖这些连接的长时间客户端。

交付物：可配置的退避、重试预算、取消和状态事件；恢复时重新发现服务并刷新
元数据；剪贴板保留基线抑制、nonce/changeCount 去重与明确的断线期间事件语义。

验收：覆盖拔插、Wi-Fi 中断、服务重启、地址变化、缺失能力和重复通知。
取消停止重试；不会隐式创建新配对，不重复执行安装、擦除等有副作用的请求。
日志与状态输出兼容，重连期间资源和队列不持续增长。

### L04：性能与资源基线

范围：单设备/多设备连接，manager 查询/停止，文件传输与长时间流。

交付物：可复现的测量脚本和结果模板；主机模拟 1、4、16 台设备，真机按可用
数量执行；记录 p50/p95、吞吐、内存、任务数、socket/文件句柄和停止收尾耗时。

验收：比较改动前后结果，记录硬件、构建模式、输入与采样方法；至少执行一次
有界的长时间启停/断连测试。资源在释放和稳定阶段恢复到可解释范围；发现增长
必须归因。性能预算由基线建立并记录，避免凭空宣布吞吐或延迟承诺。

## 9. 功能工作流任务

### F01：设备级 doctor

范围：主机诊断及设备信任、Developer Mode、DDI、RSD 服务和路由诊断。

交付物：显式设备目标下的有期限能力探测及 JSON/人类报告；补充必要的平台 USB
枚举、remoted 共存和目录位置诊断，先确认现有能力再补实现。

验收：无设备也输出有效主机报告；未探测、不可达、无权限、服务缺失分别表示。
探测不隐式配对或修改设备设置；不凭 ProductVersion 推断全部服务支持。
新增设备字段查询有协议依据和 fixture，真实支持范围登记到 Q01。

### F02：完整自动化工作流

范围：复用应用管理、XCTest/WDA、日志、crash、JUnit 和结果写入能力。

交付物：Rust 可组合工作流及对应 CLI 使用入口，完成显式选择设备、检查条件、
按需安装测试应用、启动测试、收集限定日志/结果、生成报告和清理的顺序。
明确成功、失败、部分产物、取消和恢复后的重跑规则。

验收：假服务覆盖各阶段失败与取消，合成测试应用在代表性设备上跑通完整流程；
产物完整性和覆盖策略可验证，退出码供 CI 判断。不新增必要性未经证明的协议层，
不在失败重试中重复有副作用的操作；缺前置条件时给出可操作诊断。

### F03：真实客户端经过 Rust CDP 桥

范围：Rust HTTP/WebSocket 桥、可注入 WebInspector 假服务、客户端测试脚本。

交付物：固定版本的 Playwright/Puppeteer 测试工具与可重复 fixture；从客户端
连接真实 Rust 桥，再由桥连接合成 WebInspector，记录完整请求、事件和关闭顺序。
覆盖初始化、目标发现、附着、上下文选择、求值和分离。

验收：两类客户端分别产生可复现结果；缺少 `Browser.setDownloadBehavior` 等
初始化语义时列出协议证据与真实实现方案。独立假 CDP 端点探针仅作请求发现工具，
不得替代本任务。客户端要求不存在可实现的对应保证时返回明确错误并登记范围。

### F04：CDP 协议域兼容

范围：在 F03 的实际客户端框架下逐域推进，保留既有显式不支持行为。

| 子任务 | 交付与验收 |
| --- | --- |
| F04-1：JSContext/Runtime | 上下文建立、销毁、求值、console 与未知 contextId 经过客户端和桥验证；保留无副作用求值等无法保证语义的明确拒绝 |
| F04-2：Page/iframe | 具有同源及跨源 iframe 的合成页面；导航、frame-tree、ID 关联和事件顺序通过客户端流程 |
| F04-3：Input/截图 | 点击、键盘、修饰键、坐标和截图编码在有证据的 WebInspector 翻译下通过流程 |
| F04-4：Fetch/下载 | 请求拦截、继续、响应、失败、取消和下载行为分别验证；无法映射的操作不返回空成功 |
| F04-5：Memory/Performance | 有证据的指标字段、单位和生命周期转换；客户端请求结果可核对，未知项明确表示 |

每个子任务交付协议证据、合成 fixture、实际客户端流程和支持范围；再补代表性
真机流程。合成数据不能自行证明未知 WebKit 字段；缺协议观察时记录具体取证任务。
F04 全部子任务验收后才能标记完成，单域完成只更新该域的兼容声明。

### F05：查看器、VNC 与双向剪贴板

范围：已有 Display/RTP/HID/剪贴板的上层工作流，分三步交付。

| 子任务 | 交付与验收 |
| --- | --- |
| F05-1：媒体解码与 Web 查看器 | 明确编码格式、解码依赖、时间戳、尺寸及内存预算；复用有界媒体流，浏览器看到可更新画面并能正常停止；保留原始 RTP API |
| F05-2：VNC 与输入 | 复用同一画面与输入层；实现受控连接、触摸/指针/键盘映射、方向变化及断线清理；记录设备不支持的输入语义 |
| F05-3：双向剪贴板 | 文本与有证据的图像格式互传；大小限制、来源标记、去重与回环抑制；重连、重复事件和拒绝格式都有测试 |

验收：主机 fixture 与代表性真机分别验证画面、输入和剪贴板；结束后媒体、HID
与监听释放。新增网络入口复用 S01–S03 的认证、Origin 与受保护传输。
先选定可跨目标平台交付的解码方案并记录许可证及打包影响，再扩大格式支持。
三步均有独立结果，不将原始编码流可读取当作查看器完成。

### F06：恢复预检与实验恢复执行

范围：现有 IPSW identity 选择、设备侧 preflight、TSS、恢复消息和分块 codec。

| 子任务 | 交付与验收 |
| --- | --- |
| F06-1：预检 CLI | 取得板号、芯片号、型号及支持版本的设备协议证据；实现 `preflight-requests` 入口，正确选择 identity，每 updater 使用所需的新连接；版本拒绝、身份歧义、预算与错误经过假服务验证 |
| F06-2：实验恢复执行器 | 为明确的机型/固件/恢复路径建立模式转换、nonce/TSS、固件传输、进度/完成与失败处理状态机；先完成一种有证据的 Update 流程，再登记 Erase/DFU 等扩展范围 |

验收：离线 IPSW、假 TSS/恢复服务覆盖预算、失败、取消、分块、nonce 变化和
迟到消息；不以 codec 完成替代恢复循环。实际 TSS 与真机恢复单独验收，记录设备
所有者明确的操作范围、合成/专用测试环境和恢复条件，不从本次计划编写推导擦除
或刷机操作。未知固件映射和设备模式转换不得用猜测字段或空成功补齐。

完整恢复缺少必要证据时，列出具体的协议、传输、机型和固件缺口，保持实验状态，
继续完成可独立验证的预检和执行器部分。真实恢复未验收时，F06 保持待验收。

## 10. 文档、交付与质量治理

### D01：文档与变更记录

交付物：同步根目录与 crate README、功能说明、构建/排障/协议/API 文档和
CHANGELOG；历史计划标明阶段记录，当前状态指向 Q01，避免在多处维护矛盾结论。
完善 PR/交付模板，记录接口变化、验证证据、真实剩余限制和任务 ID。

验收：新增公开行为在对应文档可发现，示例能够按声明条件运行；中英文关键
能力与边界一致；新功能和修复进入 Unreleased。提交按独立能力或缺陷组织，
大范围重构与新行为分开。纯文档改动检查链接、格式和事实，不为其添加产品测试。
本任务首次整改有交付记录，后续维护义务继续适用于每个实施任务。

### D02：质量指标与 1.0 准备

交付物：定期工程回顾模板，记录已验证工作流、代表性真机覆盖、发布后回归、
关键延迟、资源释放及兼容性变更；定义 1.0 候选 API/命令/输出和实验能力边界。
每次发布后核对回归，重要协议或依赖升级时核对兼容性，每月更新一次工程回顾；
若开发节奏变化，明确调整回顾频率并保留上一轮结论。

验收：支持范围对应实际证据；必需 CI/制品检查阻止不满足条件的发布；至少一次
跨代表平台/设备的候选工作流回归和长时间运行检查完成。每个公开稳定接口有
错误、资源所有权、取消与迁移说明。发布准备文档记录未完成任务与实验范围。
1.0 准备完成不等于自动发布，也不等于本计划其他功能任务全部完成。

## 11. 统一验收与任务交接

产品修改先执行相关 parser、codec、假服务或 CLI 行为检查，再运行统一入口。
入口落地前使用以下主线检查，Python 解释器配置见现有 Python 构建说明：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --exclude ios-py --all-features --locked
cargo test -p ios-py --no-default-features --locked
cargo +1.80 check --workspace --exclude ios-py --locked
```

B01 落地后增加自动化最小 feature 编译与相应测试；B02 落地后增加默认 feature
文档测试。Python 修改补 wheel 构建/安装/导入，FFI 修改补头文件与 C 调用检查。
跨平台文件和传输行为在对应系统验证，Linux 结果不代替其他系统或设备证据。
依赖升级记录 lockfile 与 MSRV 影响；协议/状态机测试关注可观察行为与异常路径。

每个任务完成时追加以下交付记录，并更新总览状态：

| 字段 | 必填内容 |
| --- | --- |
| 任务与范围 | 任务 ID、完成的子任务、受测代码提交与日期 |
| 实际变更 | 文件/接口及用户可观察行为，必要的迁移说明 |
| 主机验证 | 执行命令、平台、通过/失败/忽略及原因 |
| 客户端/真机证据 | 客户端版本、设备条件、脱敏证据索引或缺失条件 |
| 剩余事项 | 精确列出未覆盖范围与下一步，不能以计划充当通过 |
| 状态 | 待条件、待验收或完成，并写明依据 |

当前执行范围完成的判定是：除 Q04 外30个主任务及其列出的功能子任务均满足各自验收，
必要真实证据已有索引，当前文档与发布合同一致。条件不足的任务可以继续排队，
但不能从清单中静默删除或被其他测试结果代替。后续范围调整须明确修改对应任务、
原因和验收边界，保留既有交付记录。

## 12. 建议的首轮实施顺序

1. 完成 B01、B02，先关闭已复现的最小接入与默认文档问题。
2. 完成 B03、B04，统一验证与发布门禁，落实 D01 首批文档整改。
3. 建立 Q01，并推进 S04、Q03；建立可靠的能力、输出和证据基线。
4. 完成 S01–S03，随后验收远端管理与浏览器入口的安全合同。
5. 推进 Q02；独立开展 A01 和 R01–R03，再落实 R04、A02。Q04已按用户要求排除。
6. 完成 L01–L04，以模拟多设备和代表性真机结果检验稳定性。
7. 按实际依赖推进 A03、F01–F06，逐子任务交付，持续更新 D01–D02。

实施状态以第 3 节总览和第 14 节交付记录为准；历史基线不作为新实现的通过证据。

## 13. 目标模式执行与大类提交规则

用户已要求在独立新会话中启动本计划：模型使用 `gpt-6.1-sol`，推理等级为
极高（`xhigh`），以持久化目标模式持续推进。每完成一个大类，必须完成验收、
本地 Git 提交和计划更新，再开始下一大类。目标不预设 token 或时间预算。

2026-10-06 用户最新授权允许子代理；此前禁止子代理的交接要求按此更新。
审查和梳理可并行，所有 Rust 编译、测试及验证由主代理串行执行，避免并行构建。
任何项目验证进程仍运行时不编辑源码；真实退出后再审查、提交及进入下一实现。

大类按可交付工作包组织，以解决字母分组之间的交叉依赖：

| 顺序 | 大类 | 对应任务 |
| --- | --- | --- |
| 1 | 构建、接入与发布 | B01–B04，D01 首批相关文档整改 |
| 2 | 能力证据与管理面安全 | Q01–Q03、S01–S04 |
| 3 | 模块结构与公共契约 | R01–R04、A01–A02 |
| 4 | 生命周期与性能（Q04已排除） | L01–L04 |
| 5 | SDK、设备诊断与自动化工作流 | A03、F01–F02 |
| 6 | CDP 客户端与协议域 | F03–F04 |
| 7 | 查看器、VNC 与双向剪贴板 | F05 |
| 8 | 恢复预检与实验恢复执行 | F06 |
| 9 | 文档收尾、质量指标与发布准备 | D01–D02 的最终验收 |

一个大类可以有多个聚焦的实现提交。其出口必须：

1. 完成当前可执行范围的实现，运行相关检查和规定的主线验证，审查实际 diff。
2. 按明确文件范围提交本类代码及专题文档，避免混入其他任务或用户的无关变更。
3. 更新本文任务总览和交付记录，写明实际结果、准确实现提交哈希、剩余条件，
   将该计划更新作为本类收尾提交。提交说明引用对应任务 ID。
4. 核对 `git log`、提交内容与工作区剩余变更，报告本类结果，再按依赖推进下一类。

当前尚未提交的本计划是已授权的前置产物，纳入首轮相关提交。提交不包含推送、
发布、删除历史、强制改写提交或操作参考 checkout。

缺真机、平台或协议证据时，记录具体受阻子项及已交付主机部分；完成可执行部分
的提交和文档收尾后，继续独立任务。待条件项仍保持待条件或待验收，不能宣称整类
全部验证完成。后续依赖只在所需接口、工具或证据实际就绪时解除，并记录依据。

只有当前有效范围30个主任务（排除Q04）及所有列出的子任务实际达到验收条件，
新的持久化目标才能标记完成；Q04不计为完成，历史31项记录继续保留。
每个大类的提交是中间交付，不结束总体目标；真实不可消除的条件按目标模式的
阻塞规则记录，不通过重复运行同一失败或降低验收标准假装完成。

## 14. 持久化实施与交付记录

### 启动记录（2026-09-30）

- 工作目录：`/home/oslo/projects/rust-ios-device`；已读取本地忽略的 `AGENTS.MD`
  和完整计划；不提交该指南，不操作参考 checkout。
- 分支：`feat/device-service-updates-20260927`；初始 HEAD：
  `572c2973489747ee9f509e9afd78a987591258b1`。
- 初始 `git status --short` 仅有 `?? docs/engineering-roadmap-2026-09-30.md`；
  该授权前置文档纳入首类实现提交，无其他用户变更。
- 保持目标 active；按第 13 节逐类实现、验收、提交和文档收尾；不推送、不发布。
- 首类验收正在执行；本记录仅说明启动事实，完整结果和实现哈希在本类实现
  提交后追加，不提前把平台或真机条件标为通过。

### 大类 1：构建、接入与发布（2026-09-30）

实现与受测提交：`3013799b88ca710a9ca7bf42942c25a9394a8dc1`。
完整主机证据索引：[构建与发布验收](validation/build-release-2026-09-30.md)。
初始未跟踪计划已包含在该提交；提交范围为本类 30 个文件，未包含参考 checkout、
本地指南、凭据、构建制品或无关用户修改。实现提交后工作区干净。

| 任务与子项 | 实际交付与用户可观察行为 | 验收与剩余条件 | 状态 |
| --- | --- | --- | --- |
| B01：两个依赖、默认、各服务/分组/别名、常用组合 | `fileservice`/`deviceinfo` 自动启用 `tunnel`；三个服务集成测试声明 required-features；验证脚本从 Cargo metadata 自动发现全部公开和隐式 feature，保留裁剪告警日志 | Linux 66 个最小配置编译及四组测试 PASS；新增 feature 自动入 CI；告警来自 shared tunnel/MTU、pairing/H2/TLV、条件导入，full 严格 Clippy PASS | 完成 |
| B02：默认文档、封装边界、四个 README 与根目录/入门示例 | 修正私有模块路径，RemotePairing 示例显式 cfg，Python 空设备判断及 finally 释放；C 状态常量/错误清空合同和完整示例，纠正绑定能力描述 | 默认/full 各 7 doctest PASS，保留 5 个 compile_fail；3 个默认 feature 下游 Rust 示例 PASS，Python 语法及安装 API PASS，C 示例/ABI 调用 PASS；不声称设备执行 | 完成 |
| B03：统一入口、快速/完整模式、Python host/打包分离 | 新增标准库 Python 入口与细分 scope；派生解释器/libpython/prefix，失败退出且标注项目；CI、贡献指南、PR 模板使用同一集合 | Linux full PASS；失败退出与缺工具检查 PASS；macOS/Windows 的入口、库搜索路径及 MSVC 调用已配置，但尚无对应系统执行证据 | 待验收 |
| B04：必需门禁、CLI/C/wheel/sdist、版本/checksum/清单 | 三个发布任务传递依赖全部必需检查及全部候选制品矩阵；PR 可安全验证制品；取消制品缺失的 `|| true`；实际 CLI `--version`、发布库 C 调用、归档和 Python 安装验证 | Linux native 和 Python 3.9/3.13 PASS，3.12 sdist 重建/安装 PASS；工作流 actionlint 与依赖图 PASS；其他平台/ARM runner NOT_RUN；CLI registry dry-run FAIL，具体原因见下 | 待条件 |
| D01 首批文档 | 根目录中英文及四 crate README、build/getting-started/python-binding、CHANGELOG Unreleased 和 PR 模板同步 | 首批整改完成；Q01 权威清单联接和后续各类文档维护仍需持续执行 | 进行中 |

先执行的相关检查：`python3 scripts/validate.py features`、`docs`、`ffi`、
`quick --package ios-cli`、`tooling-tests`；分别通过 66 个配置、默认/full 文档、
C 动态链接/调用、529 CLI 测试和 5 个验证工具失败/门禁检查。

随后执行 `python3 scripts/validate.py full --python <本仓库 .venv/bin/python>`，
选择 CPython 3.12.13；完整集合 PASS，包含以下第 11 节主线（均为 locked）：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --exclude ios-py --all-features --locked
cargo test -p ios-py --no-default-features --locked
cargo +1.80 check --workspace --exclude ios-py --locked
```

Rust 1546 PASS、0 FAIL、1 既有 PBKDF2 ignored；Python 6 PASS。
同入口追加 feature、默认/full doctest、下游示例、C、wheel/sdist 与工具测试；全部
通过。Rust 1.97.1 / MSRV Cargo 1.80.1；Linux x86_64，不等价于其他系统验证。

额外执行 `artifacts.py native --target x86_64-unknown-linux-gnu` 与 `verify-native`
PASS；release wheel 构建后分别执行 `artifacts.py python --directory
target/validation/python-release --python 3.9` / `3.13` PASS；host 测试亦在
3.9.25/3.13.15 各 6 PASS。`actionlint` 1.7.7、`bash -n`、diff 空白检查 PASS。
期间提取规则、缺 CLI version 和 rustfmt 检查曾真实失败，修复后的复跑通过；
未将早期失败当作通过。最终输出位置见证据索引。

发布条件：core/CLI `cargo package --list --allow-dirty --locked` PASS；core
`cargo publish --registry crates-io --dry-run --allow-dirty --locked` PASS。
CLI 同命令与多包验证解析到已发布 `ios-core 0.1.15`，缺少当前新增 API，26 个
编译错误，FAIL；不是本地 workspace 构建失败，也不能凭文件清单替代 tarball 验证。
当前 0.1.13 已在 registry 存在，需 D02 发布准备选择未使用版本，同步 workspace
依赖，按 core 入索引后 CLI 验证的顺序处理；本次不发布或覆盖现有版本。

本类出口：可执行主机部分已验收并提交；B03/B04 的上述条件保留，整类不宣称
跨平台全部完成。B03 的验证接口已就绪，可以解除 Q03、R/A/L 的主机工具依赖；
跨平台验收依赖继续保持。下一类从 Q01、S04、Q03 推进，再落实 S01–S03/Q02。

### 大类 2 进度：能力清单与敏感展示基础（2026-09-30，整类进行中）

本记录为类内进度，不是第二类出口；S01–S03/Q02 尚待实施，S04/Q03 仍有主机
工作。不会因这两个聚焦实现提交推进到第三类。

- Q01 实现提交：`00dc2234a32dc7fc42bd6d06a01ab2cb0e041b1c`。
  `docs/capabilities.json` 为权威清单，118 条记录覆盖真实 CLI help 的 62 个命令、
  49 个公共服务族及 Rust/C/Python、CDP/媒体/剪贴板/恢复工作流边界。实现、
  协议源码定位和证据分开；编译 PASS 仅证明编译，聚合单元测试不认证每个工作流。
  所有条目暂为实验性；真机与兼容范围缺失显式 NOT_RUN。CI 核对源码/实际 CLI
  清单、结果与证据等级，并拒绝用主机 PASS 自动得到 stable。Q01 达到清单验收，
  后续维护持续适用于所有任务；不代表这些设备能力已真机验证。
- S04 首批提交：`62a33b0b4d37de7cfaba43ae10c8bed6ee4fa56d`。
  PairRecord、PersistedCredentials、PairedCredentials 的普通/pretty Debug
  均隐藏全部材料；新增 `ios_core::redaction::diagnostic_id`，使用每进程随机
  密钥产生关联代号，熵不可用时固定隐藏。manager/forward/tunnel 的明确标识字段
  使用该策略；Instruments 日志仅保留消息元数据/类型，不输出设备 payload。
  显式选设备输出与序列化持久化保持用途区分。合成 Debug 及关联代号测试 PASS。
- S04 仍需：CoreError/PairRecordError 的标识与路径错误、其他 Debug 设备类型、
  HTTP/CDP 及 Python/C 错误、上游 URL/设备数据和异常/清理日志的完整原始捕获
  自检；当前 `{err}` 链仍可能含标识，不能宣称全部日志已脱敏。状态继续进行中。
- Q03 已完成当前路径审查及 [共享输出合同](output-contract.md)。确认 AFC pull
  直接写目标，CoreDevice pull 在传输前截断目标，image helper 缺取消 guard；
  接下来统一临时文件/提交/取消收尾并覆盖失败与 hash。macOS、Windows 及真实
  UNC 条件继续分别登记，不能凭 Linux 关闭这些子项。仅文档审查不算行为实现。
- D01 已给根目录中英文与历史审计/计划加入 Q01 权威入口；D02 开始
  [月度质量模板](quality-review.md)，记录真机、制品、性能、资源和 1.0 合同的
  NOT_RUN/待条件；不宣称 1.0 就绪。

验证：先运行 `cargo test -p ios-core --all-features --locked debug_`（5 PASS，
含三个新增凭据测试），`... redaction`（2 PASS），能力清单/真实 release CLI 对照
（118 记录 PASS），工具测试（10 PASS）；然后运行完整 `validate.py full`，选择
本仓库 CPython 3.12.13，日志 `/tmp/ios-q01-s04-foundation-full.log`。
第 11 节全部主线 PASS：Rust 1551 PASS、1 既有 PBKDF2 ignored，Python 6 PASS；
66 个 feature 配置/四组测试、默认/full 各 7 doctest、3 下游示例、C、wheel/sdist
重建安装、工具测试和真实 `cargo run -p ios-cli --locked -- --help` 清单均 PASS。
`actionlint` 与 `git diff --check` PASS。受测代码为上述两个相邻实现提交的最终
组合；无设备操作、远端 CI、推送或发布。提交后剩余仅本进度计划和 Q03 合同文档。

### 大类 2 进度：Q03 宿主输出保护（2026-09-30，整类仍进行中）

实现提交：`9e568032ecd34421ebe02ff810e9ed035e09a311`。
受测代码为该提交；验证与边界见 [宿主输出证据](validation/host-output-2026-09-30.md)
和 [输出合同](output-contract.md)。这是类内交付，S04、S01–S03、Q02 尚未
满足第二类可执行范围，继续本类，不进入第三类。

| Q03 子项 | 已交付行为与验证 | 剩余条件 |
| --- | --- | --- |
| AFC/CoreDevice pull 和 image 输出 | 下载完成前不截断既有目标；私有临时文件、flush/sync、提交及取消 guard；无 force 原子 no-clobber，force 拒绝符号链接和目录 | 真机 AFC/RSD/image 流程另登记；macOS/Windows 执行尚无证据 |
| 备份接收、metadata、regular-file copy | 新增私有 staging helper，保留 root/layout 检查；成功后逐文件替换；EOF/取消/逐文件 remote error 保留原内容，filtered placeholder 仅成功后提交 | 整个备份不是事务；保留 DeviceLink remote error 后继续的既有协议，循环成功不认证每个文件；真机及跨平台待验收 |
| JUnit 与 trace | 同目录临时文件名不再追加到目标名，合法 255 字节组件可用；JUnit 共享替换，trace collect 排他目录提交，清理失败明确报错/记录类别 | macOS 排他 rename 和系统别名、Windows readonly/Unicode/long/verbatim/收尾需要对应系统结果 |
| 失败、取消与资源边界 | 合成 EOF/取消保留旧文件；并发创建目标不被覆盖；NUL 不写路径前缀；Linux 内核 EFBIG 写失败在独立子进程验收，不改父进程限制；成功样本大小/SHA-256 一致 | 文件系统不支持 hard link/排他 rename 时明确失败；开始后的提交可完成，不能承诺取消回滚、断电持久性或多文件事务 |
| 路径与文档 | 区分设备/宿主路径，沿用私有根和父目录信任边界；MSYS 既有排障保留，根目录中英文/CLI README 与 CHANGELOG 同步 | 真实 UNC 为 NOT_RUN，需明确授权 share/fixture；不声称 dirfd/openat2 并发目录替换保护 |

先运行相关检查（全部 locked）：`cargo test -p ios-cli cmd::file::tests`
30 PASS；`cargo test -p ios-core --all-features backup2` 64 PASS、1 既有
PBKDF2 ignored；`... ostrace` 27 PASS；`... fs_replace` 3 PASS；
`... staged_file` 3 PASS（含隔离子进程的真实写失败）；`... --test testmanager junit`
3 PASS。完整入口在补充写失败和成功提交解除 guard 后复跑：

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

日志 `/tmp/ios-q03-final-full.log`，Linux x86_64 / Rust 1.97.1、MSRV 1.80.1 /
CPython 3.12.13。第 11 节全部主线 PASS：Rust 1564 PASS、0 FAIL、1 PBKDF2
ignored；Python host 6 PASS。66 个 feature 编译/四组合测试、默认/full 各 7
doctest、3 默认 feature 下游示例、C ABI 调用、wheel/sdist 重建安装、10 工具测试
及 118 条能力清单/实际 CLI help 均 PASS。首轮 lint 曾因移除 error-preview
后留下未用常量 FAIL，已删除常量；最终严格 lint PASS。不抹去该失败。

参考 DeviceLink remote-error 行为从 `raw-projects/pymobiledevice3` 只读核对，
实际参考 HEAD 记录在 `/tmp/ios-q03-pymobiledevice3-ref.txt` 和
`/tmp/ios-q03-go-ios-ref.txt`，符合 AGENTS.MD 的树外记录要求。检查参考 checkout
时观察到 Python 的未跟踪 `uv.lock` 和 Go 的 `go.mod` 修改，均原样保留；本轮
没有在参考 checkout 执行安装、测试或写入。主工作区实现提交后干净，仅随后新增
本交付记录/证据/质量文档；无实际设备、推送、发布或远端 CI 操作。

Q03 状态为待验收：主机接口/合同已就绪，可供后续 R01 等主机依赖使用；上述
跨平台/真实 UNC/设备条件不自动解除。S04 的后续错误链和 Debug 审查继续执行。

### 大类 2 进度：S04 错误与发现诊断（2026-09-30，S04 仍进行中）

实现提交：`c8ed4c9c252d50f493d1117c9089982c384aa3aa`。
受测代码为该提交；[格式化验收证据](validation/error-diagnostics-2026-09-30.md)
区分定点格式化回归与尚未验收的完整输出边界。本次不关闭 S04，不推进第三类。

- CoreError/MuxError 手写 Debug 只保留错误类别、IO kind/os code 或关联代号，
  不渲染任意协议/来源数据；DeviceNotFound Display 使用进程内代号。
- PairRecordError 普通/pretty Debug 和 Display 隐藏标识、路径、解析材料及自定义
  IO 文本；Read 仍保留 IO 类型和 source，Rust 字段/变体可供明确的程序调用使用。
  缺少配对记录及 direct-RSD 目标的连接错误也不再拼入原始 UDID/IP。
- DeviceInfo、DeviceEvent、MuxDevice、BonjourService、MdnsDevice、ConnectOptions、
  PairedMobdev2Device 与内部 LockdownTransport 的 Debug 隐藏设备身份、网络地址、
  TXT 内容或配对文件路径。显式选设备字段/serde 输出保持用途；测试同时验证该合同。
- PersistedCredentials::list 不打印目录或任意 IO 文本；SECURITY/CHANGELOG 同步
  本批范围。RemotePairingRecord::list 的目录日志尚未整改，不能把前者结果推广至后者。

先运行 `cargo test -p ios-core --all-features --locked debug_` 9 PASS（含四个新增
格式化测试），`... pair_record_error_diagnostics` 1 PASS，`... device::tests`
47 PASS；单独 `rustfmt --edition 2021 --check crates/ios-core/src/device/tests.rs`
PASS，因该文件通过 include! 接入。设备相关用例是合成协议/主机测试，不是真机证据。
窄回归曾因缺配对记录错误仍含原始 UDID FAIL；沿实际 Unsupported 构造路径整改后
复跑 PASS，保留错误类型与阶段。最终执行完整入口：

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

日志 `/tmp/ios-s04-error-debug-full.log`。Linux x86_64 / Rust 1.97.1、MSRV 1.80.1 /
CPython 3.12.13；第 11 节全部主线 PASS。Rust 1569 PASS、0 FAIL、1 既有 PBKDF2
ignored；Python 6 PASS；66 feature/四组合、默认/full doctest、3 下游示例、C ABI、
wheel/sdist 重建安装、10 工具测试、118 能力条目/真实 CLI help 均 PASS。
审查明确文件范围后提交；提交后主工作区干净，参考 checkout 的既有变更原样保留。
无设备操作、推送、发布或远端 CI。

真实剩余主机工作：任意 Protocol/IO Display 与 source 链的默认渲染，CLI main/
eprintln、manager HTTP/WS、CDP/Selenium、Python 异常及全部诊断 log site；上游
URL/认证材料/设备 payload 的异常、timeout 和 cleanup 路径；原始 stdout/stderr
捕获自检及脱敏副本分开验收。源码审查索引在 `/tmp/ios-s04-log-sites.txt`（876 个
log/output 匹配位置，含显式产品结果，并非 876 个已证实泄漏）。仍需逐项区分
用户请求的设备选择/结果输出与默认诊断；定点 Debug 无命中不证明所有原始日志安全。

### 大类 2 进度：S04 自动错误边界与 Q02 捕获基础（2026-09-30，仍进行中）

实现提交：`395b90bd206f746ceb80af736402b316cc731a5f`。
受测代码为该提交；[边界验收证据](validation/diagnostic-boundaries-2026-09-30.md)
和 [诊断合同](diagnostic-contract.md) 记录行为、兼容变化和剩余范围。本次是类内交付，
不关闭 S04/Q02，不推进第三类，也不结束持久化目标。

- 新增公开 `diagnostic_error` 借用视图：阶段由调用方固定提供，错误只展示类型类别、
  可得的 IO kind 与进程内随机盐关联代号。哈希最多消费 4096 格式化字节和八层来源，
  不拼装中间错误字符串；循环来源受限。原始错误类型、字段和 source 仍供明确 API 使用。
- CLI main 不再由 anyhow Termination 打印任意来源链；保留运行错误退出 1、参数错误
  退出 2，参数值不回显，帮助/版本正常展示。ForceRequired 穿过 context 仍给出固定
  `--force` 提示。UDID/P12/Wi-Fi 密码环境值不进入帮助输出。
- manager 操作错误、WebDriver 异常及 CDP 桥错误使用固定阶段与安全错误视图；
  manager 缺失/不支持目标的错误体改用 `device` 关联代号，不回显 `udid`/IP。
  CDP 设备协议错误保留 id/sessionId/数值 code，替换 message 并丢弃任意 error data；
  显式 Runtime 结果、exceptionDetails、console 和页面数据不改写。
- Python RuntimeError/ValueError 类型保持，异常不回显协议正文/非法模式值；repr
  隐藏地址，connect_info/属性保留实际选择数据。C 仍以数字状态报告错误，增加合成
  UDID/无效 UTF-8 的无设备失败检查，并实际捕获 C stdout/stderr。
- 逐点整改 backup rollback/cleanup、DDI、RSD、DTX、活动 trace、userspace、forward、
  prepare、screenshot、syslog、manager、dproxy 等诊断。路径/上游/端点/任意名称使用
  关联代号，错误不打印正文；RemotePairingRecord::list 的遗留目录日志已整改。
  dproxy 自动进度不再展示协议 summary，明确请求的原始录制仍保持数据用途。
- `scripts/diagnostics.py` 在 256 KiB 总 RAM/时间预算内捕获实际 CLI；原始检测和
  脱敏副本独立记录，报告无原始输出/argv/env。退出码、阶段、非空输出、timeout、
  overflow 和 incomplete 都参加判定。六个工具回归覆盖变体、双流、空成功、洪泛、
  timeout 与 Unix 本轮进程组的继承管道清理。已接入 full/CI 的强制 diagnostics。

先跑错误视图两项定点测试，CLI 538 PASS、Python 7 PASS、捕获工具 5 PASS 与十个
CLI 原始捕获 PASS；新增 CDP/ Python repr/C 路径纳入最终完整主线。开发期曾出现
Parser trait 导入、anyhow 到 StdError 的转换、测试 router 名称和未启用 reqwest json
方法的编译失败；已改用现有导入、as_ref、实际路由及序列化 body，不增加依赖。

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
python3 scripts/validate.py tooling-tests
python3 scripts/diagnostics.py
/tmp/ios-validation-tools/actionlint .github/workflows/ci.yml
```

Linux x86_64 / Rust 1.97.1、MSRV 1.80.1 / CPython 3.12.13。
主线 `/tmp/ios-s04-boundaries-full.log` 全部 PASS：Rust 1577 PASS、0 FAIL、1 既有
PBKDF2 ignored，Python 8 PASS，66 feature/四组合、默认/full doctest 七项各自、
三个下游示例、C ABI/原始捕获、wheel/sdist 重建安装、118 能力条目/真实帮助均通过。
完整入口工具测试 15 PASS；随后添加空成功拒绝自检，复跑 tooling-tests 16 PASS
(`/tmp/ios-s04-boundary-tooling-final.log`)。最终 CLI 十例捕获
(`/tmp/ios-s04-process-final-capture.json`) 均原始/副本零命中且合同通过；不以副本
零命中推导原始安全。actionlint/diff --check PASS。
按 AGENTS.MD 的 Python 变更验证要求，从 crates/ios-py 使用现有 .venv 设置
VIRTUAL_ENV/PYO3_PYTHON/PATH 执行 `uvx maturin develop --locked` PASS
(`/tmp/ios-s04-maturin-develop.log`)，仅安装本地 editable 扩展，无发布。

真实剩余：框架自动 Query/Json/Path 拒绝回复、剩余输出点及派生 Debug 审查、实际
timeout/cleanup 日志捕获、协议/设备真机证据；S01–S03 认证/授权/Host/Origin/
受保护联邦仍待实现。Q02 的完整代表矩阵、受测提交/二进制/参数与安全证据持久化、
跨平台进程树管理和全部真机流程仍待；当前捕获基础只解除该工具的主机接口依赖。
Windows/macOS 未运行本批，不能据 Linux/合成 WS fixture 认定设备或平台验收完成。
实现提交后工作区干净；无设备操作、推送、发布、历史改写或参考 checkout 修改。

### 大类 2 进度：S04 框架拒绝、Debug 与生命周期日志验收（2026-09-30）

实现提交：`1953f8b4273ea61703c22245e4da5b2e2bedc591`。
受测代码为该提交；[本轮证据](validation/http-debug-diagnostics-2026-09-30.md)
连接之前三个 S04 实现批次与当前 [展示合同](diagnostic-contract.md)。S04 达到所列
合成秘密/主机诊断验收，状态完成；不将这个结果升级为设备工作流、平台安全或
访问控制认证。Q02 的设备观察和 S01–S03 的新增接口仍要持续执行同一展示策略。
第二类仍进行中，接下来实施 S01，再按实际接口落实 S02/S03/Q02。

- manager、CDP、Selenium 的框架 Query/Json/Path/握手等未审查错误体直接丢弃，
  不先读取或格式化；保留 HTTP status、Allow、HEAD 空体，加入固定 HTTP 阶段、
  数值类别、随机请求关联代号和 no-store。已审查错误 helper 使用服务器内部
  response extension 标记，客户端 header 无法豁免。真实 HTTP fixture 覆盖三种回复形状。
- 38 个具名 protocol/service 错误和 service_error! 生成类型统一安全 Debug；
  原变体、字段、Display/source 保留。FullPairRecord、SupervisionIdentity、
  VerifyPairSession 隐藏全部材料；TunnelInfo 地址使用关联代号，包含无 tunnel 构建。
  typed-value Debug/serde 属于明确数据访问，不能被自动诊断调用；不是全类型禁用 Debug。
- 审查剩余 stderr/tracing 点：文件/配对/安装/DDI/dproxy 进度、sysmontap、
  fetchsymbols 解析失败、JS shell 协议错误及剪贴板 promise 错误不渲染原文。
  明确请求的脚本异常、剪贴板值、选择数据与录制仍保留产品用途。新增 derive guard
  防止 owned thiserror::Error 回退到 payload Debug；它是源码约定检查，不声称 AST 证明。
- 256 KiB 原始 RAM tracing 捕获有故意输出合成秘密的正向控制和 UTF-8 overflow
  自检；真实备份回滚失败与 userspace TCP 的部分前导 EOF/超时要求非空实际阶段、
  安全类别/关联和零原始命中。异步用例在本轮独立、限时子进程安装全局 subscriber，
  stdout/stderr 丢弃，取消/超时只终止该子进程，不修改并行父测试的全局 tracing。

先跑相关检查：core 全 feature lib 953 PASS、1 既有 ignored；CLI 542 PASS；
tooling-tests 17 PASS。最终完整入口（锁定 CPython 3.12.13）为：

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint .github/workflows/ci.yml
git diff --check
```

日志 `/tmp/ios-s04-http-debug-final-full.log`，Linux x86_64 / Rust 1.97.1、
MSRV 1.80.1。第 11 节全部 PASS：Rust 1589 PASS、0 FAIL、1 既有 PBKDF2 ignored，
Python 8 PASS；66 feature 编译、四组裁剪测试、默认/full doctest 各 7、三个下游
示例、C ABI/19 字节原始捕获、wheel/sdist 重建安装、17 工具测试、118 能力条目
及十个实际 CLI 原始/副本独立捕获全部通过。actionlint 与 staged diff --check PASS。
主机 CLI 二进制 SHA-256 和具体命令见本轮证据；不归档敏感原始输出。

开发失败如实保留：测试 key fixture 长度与无 tunnel 条件编译曾失败，已修正；
严格 lint 曾拒绝 prepare 模块测试位置，移至文件末尾后 PASS。首轮完整入口
`/tmp/ios-s04-http-debug-full.log` 因并行异步 tracing 捕获为空 FAIL；局部 subscriber
与 WithSubscriber 两次完整 core 复跑仍 FAIL，未放宽阶段断言。隔离捕获后 core 全套
PASS，随后从头复跑上述 full 全部 PASS。空日志未计为安全成功。

当前无已知剩余 S04 主机修复项。真实设备、多系统和全部代表工作流的观察归 Q02
持续验收；S01–S03 的认证/授权、Host/Origin 和受保护联邦尚未交付，不能被安全
错误文本替代。实现提交后主工作区干净，仅随后新增本记录/证据/质量文档；未操作
实际设备、远端 CI、推送、发布、历史或参考 checkout。持久化目标继续 active。

### 大类 2 进度：S01 凭据文件基础与 S04 重新打开（2026-09-30）

S01 实现提交：`004b6af2c6041202a1f395d7466b7a241852e373`。
新增 `secret_file::read_secret` 和 `write_secret_new`，供后续服务端策略和客户端
凭据使用。前者返回 Zeroizing buffer，最多 1 MiB、检查实际打开的 regular file；
Unix 要求有效用户所有且无 group/other 权限，O_NOFOLLOW/O_NONBLOCK 拒绝叶子
链接及 FIFO。Windows 使用 OPEN_REPARSE_POINT 和 reparse attribute 检查；
ACL 仍依赖父目录，Windows 执行未发生，不能将文档旗标核对当平台证据。
后者私有随机 staging、sync、hard-link no-clobber，既有文件/目录/叶子 symlink
不被替换，仅清理本轮 staging；不支持 hard link 明确失败。Unix 对已有非私密
父目录报错且不 chmod，缺失私有父目录可创建。父路径仍需可信，非目录原子保证；
收尾失败可能发生在发布之后，不承诺多文件事务或断电持久性。

先运行 `cargo test -p ios-core --locked secret_file`，7 PASS；
`cargo clippy -p ios-core --all-targets --all-features --locked -- -D warnings` PASS。
随后完整入口 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` PASS，日志
`/tmp/ios-s01-secret-foundation-full.log`。Linux x86_64 / Rust 1.97.1、
MSRV 1.80.1、CPython 3.12.13：Rust 1593 PASS、0 FAIL、1 既有 ignored，
Python 8 PASS；66 feature/四组、默认/full doctest 各 7、三个下游示例、C/wheel/sdist、
17 工具测试、118 能力条目/实际帮助、十例原始诊断捕获全部 PASS。diff/staged
diff --check PASS。提交仅三个相关文件，之后剩余仅本计划进度，参考与用户变更保留。

S01 仍需服务端/客户端接入、权限与设备/端口策略、失效凭据、重新配置及实际
拒绝/成功回归；manager 当前仍未认证。本提交不关闭 S01。

检查 S01 网络依赖时发现 S04 的具体遗漏：CLI main 最高等级订阅了所有 target；
当前 tungstenite 0.24.0 源码的 TRACE 会格式化握手请求和 WebSocket frame/message，
DEBUG 会格式化 close reason。此前原始捕获未执行这种依赖协议路径。第三方日志
可能含请求 UDID/认证头和设备 payload，故重新打开 S04，而不是保留完成声明。
原 S04 验收记录作为自有输出边界的历史证据保留；总览状态已更正。
下一步限制 CLI 日志来源并以真实 WebSocket fixture 检查原始捕获；认证接入继续
依赖这一安全日志基础，不先引入令牌后忽略依赖泄漏。仍在第二类，目标 active。

### 大类 2 进度：S04 依赖日志边界补齐（2026-09-30）

实现提交：`447730f153f5c962dcc153b2875ff8a5b91f0060`。
CLI subscriber 只接纳当前已审查的 ios/ios_cli/ios_core namespace，检查组件
边界并保留选择的等级；任何 verbosity 不启用第三方日志。项目自己的失败阶段、
安全类别和关联信息继续显示。不修改依赖或其协议行为；库/SDK 的外部宿主自己
控制 subscriber，不能把 CLI 策略宣传为其他应用的全局保护。

真实 tokio-tungstenite 0.24.0 loopback fixture 发送合成 UDID URL、标记 sensitive
的 Authorization、text payload 与 close reason。两个本轮独立子进程分别使用旧式
未过滤对照和生产 subscriber：对照必须捕获真实 Request/Bearer/Received message/
close 阶段并命中原始秘密；生产配置保持协议交换与自有阶段成功，原始秘密、hex
变体、依赖 target 和近似 namespace 均不出现。256 KiB RAM、实际阶段非空要求、
三秒协议/十秒子进程期限、owned kill/reap 和丢弃子输出均参加验收；无原始归档。

先运行 `cargo test -p ios-cli --bin ios --locked logging::tests` 1 PASS（两种实际
协议子用例）；CLI 全目标严格 Clippy PASS。随后从头运行 full，日志
`/tmp/ios-s04-dependency-logging-full.log`：第 11 节及扩展门禁全部 PASS。
Linux x86_64 / Rust 1.97.1、MSRV 1.80.1、CPython 3.12.13；Rust 1594 PASS、
0 FAIL、1 既有 PBKDF2 ignored，Python 8 PASS；66 feature/四组、默认/full
doctest 各 7、三个下游示例、C ABI/19-byte 原始捕获、wheel/sdist、17 工具测试、
118 能力条目/实际帮助及十例 CLI 原始/副本捕获均 PASS。diff/staged diff --check
和 actionlint PASS。实际 target/debug/ios SHA-256：
`a54bb30e9199bbf26d8507163d635b2418707d32f987a175095bf3be587e6fb4`。

这次补充解除已证实的依赖日志缺口，S04 恢复完成状态；保留重新打开的历史，
不抹去之前捕获范围不足的事实。新认证与协议接口仍持续执行 S04 合同；Q02 的
真实设备/平台观察及 S01–S03 的访问控制保持未完成。实现提交只含五个相关文件，
提交后主工作区干净，仅随后新增本记录/证据/质量更新。继续 S01 策略及管理面
接入，不进入第三类、不结束目标；未推送、发布、改写历史或操作实际设备/参考。

### 大类 2 进度：S01 私密配置与权限准备工具（2026-09-30，仍进行中）

实现提交：`b7837cbdeade2b8812b0d85ffb2aad2104cedf33`。
新增 `ios tunnel auth create|check`，准备/检查 scoped policy；不会启动服务器、
配对设备或把策略部署到 manager。用法、schema 与部分发布合同见
[管理访问配置](management-access.md)，受测代码/命令见
[host policy 证据](validation/management-policy-2026-09-30.md)。

- 使用 OsRng 生成 256-bit token，client 文件存 64 字符 lowercase hex，配置仅存
  该 ASCII token 的 SHA-256；token、读 buffer 与编码 buffer 采用 Zeroizing，
  Policy/ClientToken Debug 不输出材料/标识。digest 用已有 subtle 比较。
- 权限显式区分 list/connect/manage_tunnels/manage_server/debug，无角色继承；
  设备和 TCP port 默认精确匹配。all_devices/all_ports 必须显式，且不隐式添加权限；
  port 0 始终拒绝。expiry equality 为过期；期限最大 365 天。
- 配置限制 64 KiB/64 grants/每 grant 128 个 selector（各 256 bytes）及 256 ports；
  拒绝未知版本/字段、重复凭据/权限、模糊 broad/list 与缺少所需 scope。空 grants
  合法且拒绝全部。create 只发布新文件；第二文件失败时返回错误并明确部分发布，
  保留第一文件和所有旧目标供检查，不按可能已被替换的路径自动删除。
- check 验证实际私有文件及精确权限，允许才输出 data:true；非法/过期凭据与
  越权返回非零。host 命令不需要 default UDID，使用 blocking worker 保持 async
  入口。CLI 新增 rand/subtle 直接依赖均已在 lock 中存在，无版本升级；MSRV PASS。
- diagnostics 实际调用 create、合法 check、越权 port check 和新增帮助，清除
  default UDID 且使 usbmux 不可用；只在本轮 TemporaryDirectory 创建私有配置，
  有界读取随机生成 token 后检查原始/独立副本，不打印/归档 token 或原始输出。

相关检查：`cargo test -p ios-cli --bin ios --locked access::tests` 5 PASS；
CLI 全目标严格 Clippy、tooling-tests 17 PASS；15 例实际 CLI 捕获全部 PASS。
随后完整入口 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` 全部 PASS，日志
`/tmp/ios-s01-auth-policy-full.log`。Linux x86_64 / Rust 1.97.1、MSRV 1.80.1、
CPython 3.12.13：Rust 1599 PASS、0 FAIL、1 既有 PBKDF2 ignored，Python 8 PASS；
66 feature/四组、默认/full doctest 各 7、三个下游示例、C ABI/19-byte raw capture、
wheel/sdist、17 tools、118 capability/实际帮助和 15 CLI 原始/副本捕获均 PASS。
actionlint、diff/staged diff --check PASS；CLI SHA-256
`cc9c48a8f806cedd66e34436cd53a64a57fab823775f10c50b65112cdda8ee5a`。

真实剩余：manager 请求认证、按 scope 过滤列表/拒绝目标和 port、配置 reload/
撤销、CLI HTTP token header 与可用访问模式、CDP/Selenium 等桥接接入、已有 TCP
转发/userspace 本地信任边界及跨平台私密文件执行。当前 manager 仍未认证，不能
用离线 policy check 当 HTTP/WS 成功或拒绝证据。先接入 manager，再落实相关桥，
随后按依赖 S02/S03；本类和目标持续进行，不因准备工具完成而结束。
实现提交仅 11 个相关文件，提交后工作区干净；无设备、推送、发布、历史或参考操作。

### 大类 2 进度：S01 manager 认证、授权与撤销（2026-10-01，仍进行中）

实现提交：`318dab89ed94e6763d0d488e3191bf1bc976ab58`。
这是实际生产 manager/CLI 接入，区别于上一条离线准备产物；此前“当前 manager
未认证”的记录描述受测历史提交，现状态以本条及
[manager 验收证据](validation/manager-access-2026-10-01.md) 为准。

- manager 启动在 listener/设备扫描前读取私密策略，默认仍绑 loopback；必须
  `--auth-config` 或明确 `--local-unauthenticated`，两者互斥。明文非 loopback
  即使有策略也拒绝；CLI 明文目标亦限制 loopback，localhost 固定 IPv4 loopback。
- 外层 admission 先于 query/path/JSON/WS extractor；所有路由及 unknown route
  缺少、重复、错误或过期 Bearer 同合同 401；有效凭据越权 403。响应 no-store，
  401 带 WWW-Authenticate，HEAD 无 body。日志/错误不携 token、策略/设备字段。
- list 按设备过滤，get/manage-tunnel 精确设备，connect 精确设备/非零端口，
  upstream/shutdown/reload 要 manage-server。无角色继承；权限检查先于下游拨号。
  自动扫描仅为有效 connect/manage-tunnels scope 建连，无此 scope 直接跳过发现。
- POST `/auth/reload` 只读启动路径；有界私密读和 schema 失败保留旧策略/会话。
  reload 串行、前后复核同一控制器的当前管理员，再原子发布快照；成功重载撤销
  所有旧 WS，包括 unchanged grants。新请求使用新凭据；expiry 也关闭既有桥。
  单调 deadline 不受时钟回拨延长，前跳每秒检查；撤销 drop 两端，不承诺 Close
  握手。等待中的 list/get/delete/start/dial 会被取消并拒绝，已完成操作不回滚。
- CLI list/stop 从 `--token-file` 有界私密读取，敏感 header，禁用代理/重定向，
  成功 body 限 1 MiB，失败 body 不读入诊断。真实 redirect 假服务确认第二主机
  没收到连接。来访 token 当前不会用于上游；独立联邦凭据仍为 S03。
- 实际 HTTP/WS 假服务覆盖全路由未认证、有效/越权/过期、list 过滤、错误策略
  保留、新凭据重载、并发/陈旧/其他控制器管理员拒绝、字节往返和双端关闭。
  审查发现 waiting list 被撤销可能空成功，已改取消并用实际上游 barrier 验收 403。
  既有 3-hop/循环/EOF/关闭/背压测试仍 PASS，旧假服务明确使用 test-only 本地信任。
- raw gate 增加生产服务捕获，只有 list/server grants，不接触设备发现；同时
  捕获实际 CLI token 客户端合法列表/越权 stop。RAM 原始输出和独立副本均零命中，
  合同/timeout/overflow/incomplete 失败不算 PASS；不写 raw 输出或秘密到证据目录。

相关检查：最终 manager 33 PASS；runtime 六项与此前 policy 五项亦包含完整门禁；
严格 CLI Clippy、17 tools PASS。首轮 full 1610 PASS 后审查补齐撤销等待路径与
两项回归，再运行 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` 最终全部 PASS，日志
`/tmp/ios-s01-manager-final-full.log`。Linux x86_64 / Rust 1.97.1、MSRV 1.80.1、
CPython 3.12.13：Rust **1612 PASS、0 FAIL、1 既有 PBKDF2 ignored**，Python 8 PASS；
66 feature/四组、默认/full doctest 各 7、三个下游示例、C ABI/19-byte capture、
wheel/sdist rebuild/install/import、17 tools、118 inventory/实际帮助、20 CLI/服务
捕获全部 PASS。actionlint、diff/staged diff --check PASS。实际 CLI SHA-256
`d272d4bc81036f6c6096736131f0c75123ba1befa38e14c7a8fbbafe63de5804`。

真实剩余：其他桥接/CDP/Selenium 的 admission 与 browser 合同、forward/runwda/
dproxy/截图等额外监听边界审查与可执行限制；userspace 20-byte proxy 在 loopback
无认证，任何本地进程可绕过 manager scope，kernel TUN 服从 OS 路由/防火墙。
本实现不宣称全部转发有凭据保护。macOS/Windows 私密文件/ACL 执行待条件；无
真机建连/管理回归。S02 Host/Origin/浏览器票据与 legacy GET 迁移仍待；S03 TLS
与每上游独立凭据仍待，当前两个 policy-mode manager 尚不能联邦。S01/整类/总体
目标保持进行中，接着处理余下入口，再按依赖推进 S02/S03/Q02。

实现提交仅 13 个相关文件，提交后主工作区干净；本条、证据索引和质量记录随后
单独收尾提交。无设备配对/DDI/TSS/擦除刷机、推送发布、历史或参考 checkout 操作。

### 大类 2 进度：S01 原生转发监听边界（2026-10-01，仍进行中）

实现提交：`e6ddf5c2bbe7a12142dfa4dfc05b6afa1133a925`。现存 userspace proxy 审查、
迁移与各入口边界见[管理访问](management-access.md)和
[监听验收证据](validation/forwarding-boundary-2026-10-01.md)。

- 复用 literal loopback/localhost 校验。forward 在设备解析前、dproxy 在服务连接/
  capture 创建前、截图 stream 在 bind/capture 前拒绝非 loopback；tuple bind 同时
  避免 IPv6 host:port 拼接错误。runwda 在测试服务/WDA 启动前验证 loopback 与两端
  非零 port；之后仍复用 forward。CLI help 明示 raw TCP/stream 的本地主机信任。
- 源码审查确认 userspace TUN 固定 `127.0.0.1:0`，20-byte IPv6/LE port 无凭据；
  drop abort listener/packet tasks。kernel TUN 遵循 OS 路由/接口/防火墙。列明所有
  原生入口，明确本地进程能绕过 manager grant，保留现有原生 wire format，不能
  宣称 token 对所有转发隔离。受保护 host transport 用于远端原生客户端路径。
- 四项有意义的 command/run 回归确认拒绝来自前置配置，而不是 usbmux 失败；
  runwda 额外测两种零 port。raw gate 新增四个真实 CLI 最大 verbosity 拒绝，
  未到设备/服务操作，24 例原始/独立副本均零秘密命中、正确非零/阶段，无丢失。
- 中英文 README 与 usage/tunnel/access/CHANGELOG 同步；修正中文 manager 示例
  需要 auth-config。此安全迁移不改变服务/20-byte 私有前导；不替代真实设备兼容。

相关 `cargo test -p ios-cli --locked nonloopback --no-fail-fast` 3 PASS 和
`cargo test -p ios-cli --locked invalid_bind_and_ports_fail_before_wda_device_startup`
1 PASS；严格 CLI Clippy、24 raw captures PASS。随后 `python3 scripts/validate.py
full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` 全部 PASS，日志
`/tmp/ios-s01-listener-full.log`：Rust **1616 PASS、0 FAIL、1 既有 ignored**，Python
8 PASS；66 features/四组、7+7 doctests/三个示例、C ABI/19-byte capture、wheel/sdist
rebuild/install/import、17 tools、118 inventory/实际帮助均 PASS；actionlint、diff/
staged diff --check PASS。CLI SHA-256
`728de4bcd69da6687922252fa924f2c04d3b9dafbe829b8c8493d3d255e053e1`。

S01 仍进行中：doctor 等客户端凭据接入、CDP/Selenium admission 与 browser 合同
尚待；macOS/Windows 私密文件/ACL 和真实设备转发待条件。S02 Host/Origin/票据/
GET 迁移、S03 每上游凭据及受保护联邦仍未交付。截图完整 F05 入口须复用届时
安全合同，当前 raw loopback stream 不算该工作流完成。实现仅 12 个相关文件，
提交后工作区干净，随后本记录/证据另做文档收尾；目标和第二类持续推进，无设备
操作、推送、发布、参考变更或历史改写。

### 大类 2 进度：S01 doctor 私密 manager 客户端（2026-10-01，仍进行中）

实现提交：`2c8c8c818669517f53161229b0840066bbdcd024`；
[验收证据](validation/doctor-manager-access-2026-10-01.md)，用法在
[troubleshooting](troubleshooting.md) / [管理访问](management-access.md)。
此交付只接入既有 host doctor，不冒充 F01 的设备 doctor。

- `ios doctor --token-file PRIVATE_FILE` 复用 S01 有界私密 token 读取、敏感 header、
  loopback 目标。继续禁代理/redirect、限制 body 1 MiB，不读取拒绝 body；report
  保留 counts 和 host scope。ClientAccessArgs 可借用，以服务并发 host probes。
- 没有凭据时，可选受保护 manager 仍 warning 并给出 token/list scope 提示；明确
  提供凭据读失败（保留安全 error category/correlation）、401/403 或凭据 probe
  deadline 则 Failed，Report.failed 导致 exit 1。其他主机检查独立保留状态。
- 真实 HTTP 假服务检查合法/错误 token、越权 403、缺少私密文件，并确认报告不
  含 token、设备、地址或远端原始错误。既有 redirect/oversized/timeout 回归保留。
- raw gate 在生产 manager 的 list/server-only 私密配置下实际运行两次 doctor：
  合法 token 的 manager check 为 ok，缺失文件为 failed；故意禁用 usbmux 使总体
  exit 1，并分别验证 JSON scope/entry，而非把这个总体非零误算全部失败/成功。
  26 captures 原始/独立副本零命中，不写出 secrets、argv/env 或 raw 输出。

相关检查 `cargo test -p ios-cli --locked cmd::doctor::tests` 6 PASS、严格 CLI
Clippy 与实际 raw capture PASS。随后完整 `python3 scripts/validate.py full
--python /home/oslo/projects/rust-ios-device/.venv/bin/python` 全部 PASS，日志
`/tmp/ios-s01-doctor-full.log`：Rust **1617 PASS、0 FAIL、1 既有 PBKDF2 ignored**，
Python 8 PASS；66 features/四组、7+7 doctests/三个示例、C ABI/19-byte capture、
wheel/sdist rebuild/install/import、17 tools、118 inventory/实际帮助、26 CLI/服务
captures、actionlint、diff/staged diff --check PASS。CLI SHA-256
`219211b96c019f1cd0f71adb7153cfe4fbac92dfbc17b6b1122561f5e2b34066`。

实现仅六个相关文件，提交后主工作区干净，本记录/证据/质量另行收尾。真实剩余
为 CDP/Selenium admission 与 browser 合同、S02 Host/Origin/票据和 GET 迁移、S03
独立上游凭据/TLS，及平台/真机条件；host doctor 没有 device trust/Developer Mode/
DDI/设备服务证据，F01 未开始。接着扩展 shared admission 到固定设备 debug scope，
再落实 S02/S03/Q02；第二类和目标保持进行中。无配对、设备启动、DDI、真实 TSS、
刷机/擦除、推送、发布、历史改写或参考 checkout 修改。

### 大类 2 进度：S01 CDP/Selenium admission 与授权会话撤销（2026-10-01）

实现提交：`0bb58a208de73b16997d8354920bbe7118c94dc5`；
[验收证据](validation/debug-access-2026-10-01.md)，合同见
[管理访问](management-access.md) / [CDP](cdp-compatibility.md)。

- CDP/Selenium 接入同一私密策略或显式 loopback local trust。配置/loopback 校验
  在 bind 与设备访问前；每个 debug 路由、metadata、unknown/extractor/WS 都检查
  bearer 和固定 UDID 的 debug grant，401/403、no-store 与各自安全错误 envelope。
  其他角色、设备范围不隐含调试权限；localhost/IPv6/port 0 metadata 使用真实监听。
- 两桥 POST `/auth/reload` 只需明确 manage-server 并读启动文件；失败保留旧策略，
  原子成功撤销全部旧租约，返回 host receipt。HTTP 等待 body/device 与 CDP WS
  在 expiry/reload 取消；browser page owner Drop abort 子任务，避免 JoinHandle detach。
- Selenium 存储保留创建者租约，原授权过期/重载先从 map 移除再限时清理；另一
  有效 Debug token 无法继续已过期会话。middleware 使用解码 session_id 防绕过，
  同时取消当前请求。Host resource 假服务验证 pending cancellation、清理超时/
  Drop/无持有环；没有构造假的 Apple runtime 或声称 Remote Automation 成功。
- 生产 page WS 在持有 page lock 的 fixture 中合法 101，再过期/重载关闭，保证
  没到设备连接；生产 WebDriver partial body 等待被重载撤销为安全 403。实际 CLI
  最大 verbosity 增四个 startup 拒绝及两桥 metadata/reload 捕获，共 32 例，原始/
  独立副本零秘密命中。两桥 capture 受控 SIGTERM，明确验 -15，而非设备清理 ACK。

相关 WebInspector 27 PASS、manager 33 PASS、严格 CLI Clippy、17 tools、32 captures
PASS。随后 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` 全部 PASS，日志
`/tmp/ios-s01-debug-full.log`：Linux x86_64 / Rust 1.97.1、MSRV 1.80.1、Python 3.12.13，
Rust **1627 PASS、0 FAIL、1 既有 PBKDF2 ignored**，Python 8 PASS；66 features/四组、
7+7 doctests/三个示例、C ABI/19-byte capture、wheel/sdist rebuild/install/import、
17 tools、118 inventory/实际帮助、32 captures、actionlint 与 diff/staged diff --check
PASS。CLI SHA-256 `30f1e22f1a859dd1586bb53bac7a6b4b4ec6f069f632651453a2903d63d3cd8e`。

实现只含 11 个相关文件，提交后工作区干净；本记录、能力证据与质量另做文档
收尾。S01 不整体关闭：平台私密文件/ACL 与真机建连/管理/会话撤销待条件，独立
联邦凭据接入属随后 S03。active Debug grant 仍覆盖整个设备，可使用有效会话，
不宣称 per-user isolation；已执行操作不回滚。100 ms host cleanup 不代表 Apple
teardown ACK，orderly lifecycle/session quota 留给 L01/F04；F03 Rust-bridge 客户端
工作流仍无证据。S02 Host/Origin/浏览器握手/GET 迁移及 S03 仍待，接着按依赖推进。
第二类和总体目标持续进行；无设备操作、推送发布、参考修改或历史改写。

### 大类 2 进度：S02 HTTP 方法与 Host/Origin 主机合同（2026-10-01，仍进行中）

实现提交：`4f868548fba6423e4060c0a1bb681a12d5222822`；
[合同与迁移](http-boundary.md)，[验收证据](validation/http-boundary-2026-10-01.md)。

- manager/CDP/Selenium 在 credential/extractor/device 前统一检查 actual listener
  IP/localhost + bound port、URI authority/scheme、单一 Host/Origin；拒绝 proxy
  authority spoofing、null/不允许来源、userinfo/path/query/fragment/wildcard/
  非法端口。原生客户端可无 Origin，仍需原 credential/scope。IPv6 真实 HTTP PASS。
- `--allow-host HOST:PORT` / `--allow-origin ORIGIN` 独立有界 whitelist（各最多
  32、authority 最多 512 ASCII bytes），启动 bind/monitor 前 validate；不代表
  凭据、CORS 或 TLS，不隐含另一维度。实际 HTTP/WS 计数 fixture 证明拒绝未到
  handler；生产三服务捕获验边界拒绝、同源原生 header 和输入不泄露。
- POST `/shutdown` 与 POST `/start-tunnel` JSON 替代状态变更 GET，保留既有 grant/
  transport/response。GET/HEAD 经授权仅 405/Allow POST 提示，到 **2026-11-01
  00:00 UTC** 后为 410 tombstone；无兼容 GET mutation。原query不解析/回显；
  CLI list GET/stop DELETE 已正确。host验证无shutdown/建连副作用、POSTnotify正向
  控制、越权设备与 JSON enum 安全拒绝，没有成功创建真机 tunnel。
- 审查修正 Authority::port 对越界返回 None 的默认端口误用；还发现 global CLI
  原先先选择首设备才进 debug 校验，现 CDP/Selenium 要求 --udid/IOS_UDID，缺少
  时不做发现。此前 S01 fixture带显式selector未覆盖省略路径，保留旧记录并补范围
  说明。primary RFC 9110/6455、WHATWG 浏览器接口依据写入合同，不以native WS当
  browser PASS。ticket/bootstrap/CORS 仍未实施。

相关边界 4 PASS、manager 35 PASS、debug 27 PASS、explicit-selector 1 PASS、严格
CLI Clippy、17 tools、37 raw captures PASS。完整 `python3 scripts/validate.py full
--python /home/oslo/projects/rust-ios-device/.venv/bin/python` 全部 PASS，日志
`/tmp/ios-s02-http-full.log`：Linux x86_64 / Rust 1.97.1、MSRV 1.80.1、Python 3.12.13，
Rust **1634 PASS、0 FAIL、1 既有 PBKDF2 ignored**，Python 8 PASS；66 features/四组、
7+7 doctests/三个示例、C ABI/19-byte capture、wheel/sdist rebuild/install/import、
17 tools、118 inventory/实际帮助、37 captures、actionlint 与 diff/staged diff --check
PASS。CLI SHA-256 `11b7229d9890d251548fc3e2f6b3ded6edda3e1ff29c1a0bd3b167e3aba33e5c`。

实现 14 个相关文件，提交后工作区干净；本记录/证据/质量另做文档收尾。S02/
第二类/目标仍进行中：下一步短期单次 browser WS ticket、bootstrap/CORS 与实际
浏览器验收；S03 独立上游凭据/保护传输随后，平台/真机待条件。全部任务子项
保留，继续独立主机范围，无设备启动、配对/DDI/TSS/刷机擦除、推送发布或参考/
历史修改。

### 大类 2 进度：S02 浏览器 bootstrap、单次票据与真实浏览器主机验收（2026-10-01）

实现提交：`826ebf3312e54fb52651969ad5d45fbee914aa5b`；
[浏览器合同](http-boundary.md)，[验收证据与重跑命令](validation/browser-access-2026-10-01.md)。

- manager/CDP POST `/auth/ticket` 发行 30 秒、单次、32-byte OS random 的握手；
  registry 只留 SHA-256，最多 512 pending、body 16 KiB、target URI 2048 bytes。
  管理 scope 不隐含 connect/debug；精确 device/port 或固定 debug device 才可发行。
  Browser JSON origin 必须是本来源，native bootstrap 只能选已允许来源。
- 绑定本监听器 registry、normalized approved Origin 和完整 path/query（目标/
  options/hop/order）；GET+WS upgrade+canonical ticket 一次消费且从 routed URI
  移除。重放/并发第二次、缺 Origin、target/origin 替换、重复/encoded alias、
  header+ticket、长期 token URL、过期均拒绝，错误不反射凭据/选择器；无 cookie
  认证。Registry quota 明确 429、entropy 失败 503。Reload 撤销旧票据和连接，
  失败 reload 保留原 snapshot；已连接 lifetime 是 grant，不是握手 30 秒。
- 三服务 shared CORS：允许来源 exact allow-origin/Vary，known-route OPTIONS
  仅提供 GET/HEAD/POST/DELETE 与 Authorization/Content-Type metadata，不执行或
  授权实际请求；无 cookie credentials。Selenium 没有 WS，不新增 ticket route。
  native 无 Origin bearer 保持原合同，explicit local trust 仍信任本机进程。
- 实际 Chromium `151.0.7922.34` / Playwright-core `1.63.0` fresh context，原生
  fetch+CORS+WebSocket，没有自定义 WS header：两组各 **16** 检查 PASS，覆盖
  bootstrap/精确 scope/错误来源/长 token URL/target tamper/replay、真实经过 Rust
  production manager 的 binary bytes、grant 到期及 reload 关流和 pending 票据失效。
  自有 TCP/upstream fixture 检查既有项目 preamble 与 EOF，无 auth/ticket 上游转发。
  CDP page 生产路由票据 101/revoke PASS，但 page lock 阻止设备连接，非 CDP 工作流。
- Browser child stdin 配置、私密 fixture token、bounded raw stdout/stderr ≤64 KiB
  each、Linux 自有 process group cleanup；检查原始 token/device/64-hex ticket 不泄漏，
  只持久化计数/版本/结果。不启用浏览器 console/HAR/video；浏览器开发工具及外部
  proxy 仍可能记录短期 ticket URL，合同明确限制日志及 HTTPS/WSS 的 S03 依赖。

相关 manager **36 PASS、1 browser ignored**，debug **28 PASS**，shared access
**21 PASS**，显式 browser gate **1 PASS（2 variants ×16 checks）**，CLI strict
Clippy、17 tools PASS。完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` PASS，`/tmp/ios-s02-tickets-full.log`：
Linux x86_64 / Rust 1.97.1 / MSRV 1.80.1 / Python 3.12.13，Rust **1642 PASS、0 FAIL、
2 ignored**（既有高成本 PBKDF2；浏览器必须配依赖单独执行，已另 PASS），Python
**8 PASS**；66 feature builds/四组、7+7 doctests/三个 snippets、C ABI/19-byte capture、
wheel/sdist rebuild/install/import、118 inventory/实际帮助、37 raw CLI/service
captures、actionlint 与 diff/staged check PASS。37 cases 内新增 CDP 实际发行票据与
拒绝使用其 plain HTTP/长期 URL、manager admin 无 connect 不能发行；generated
ticket variants 加入 raw 检测，credential JSON 成功发行是显式 API data。
CLI SHA-256 `5263e21510a3b134d25e1a890f4e8bd0840754266c06dd2e856a5a851f13c33b`。

18 个相关实现/合同文件已提交，工作区干净；本记录/证据/质量另作收尾提交。
S02 既有入口 Linux host 范围已验收，任务继续保留 F05 新入口适配与平台/实际
设备工作流待条件；S01 平台/真机与 S03 上游证据不被此门禁代替。F03 真实 CDP
客户端经过 Rust/Apple 的初始化与页面流程仍未验收。第二类和持久化目标保持
进行中，文档收尾后继续 **S03 独立上游凭据与保护传输**，再推进 Q02 余下范围。
无配对、DDI、真实 TSS、刷机/擦除、推送/发布、历史改写或参考 checkout 修改。

### 大类 2 进度：S03 受保护联邦与独立上游凭据（2026-10-01）

实现提交：`39fd900b0785ee0baa893c6f97afacbba90f9e74`；
[传输/迁移合同](protected-federation.md)，[验收证据](validation/protected-federation-2026-10-01.md)。

- `tunnel serve --upstream-config PRIVATE_FILE` bind/monitor 前私密读取 version-1
  配置及每个独立 token/optional CA；64 KiB 配置、最多 32 normalized roots、URL
  2048 bytes/path4096、CA64 KiB/16 PEM certs，unknown/duplicate/unsafe scheme/
  userinfo/query/fragment/non-root/zero-port 均失败。相对文件路径基于配置目录；
  既有 private-file/permission/symlink 与 zeroizing token 机制，无 URL/argv/env token。
- Native upstream HTTP(S)/WS(S) 同一 verifier/root store，显式 ring provider，
  WebPKI roots 与每上游独立附加 CA；不关闭 chain/validity/hostname 校验。
  只接受 literal loopback 明文，拒绝 remote/DNS HTTP（旧 localhost 改 literal IP）；
  native listeners 仍 loopback，由独立 verified SSH/TLS host forward 提供远端入口。
  提供真实配置/Host/Origin/proxy边界说明，不声称工具自动建立 SSH/反向代理。
- 每个请求头绑定 configured scheme/host/effective port，incoming bearer/browser
  ticket 和 listing 元数据不成为上游凭据。HTTP no-proxy/no-redirect，WS 不 follow/
  retry redirect；metadata next-upstream 仅交给下一已注册 manager 作为 selector，
  不直接拨号。启动配置不可通过 HTTP 改 token/CA；incoming reload 不重读上游文件，
  修改上游材料需 restart，文件编辑不自动撤销既有流，合同明确。
- 真实 HTTPS/WSS host fixture：正确 CA 能收发；无信任/过期/wrong-name 失败且
  HTTP 后端未收到凭据；上游 A 附加 CA 不授予 B。HTTP/WS 302 不联系 redirect目标；
  独立配置/Debug/原始 header scope 验收。两 production managers 经 TLS 和第三个
  owned leaf 使用不同凭据，byte/selected root/hop 保持；中间 reload 使两端 EOF。
- 修正 federation 以前吞错误为 empty success：aggregate/selection 的 credential/
  cert/redirect/body/transport 失败明确 safe 502；未知 registered selector 保持 400。
  local `/tunnels` 仍本地view；aggregate `/` 要求所有 attempted upstream成功。
  原有循环/hop、EOF、close、backpressure与 scope 测试保留并通过。
- CLI启用 pinned tokio-tungstenite 0.24 TLS feature；lock只增加 webpki-roots
  **0.26.11 compatibility wrapper → existing 1.0.6**，无既有版本升级。显式TLS
  fixtures public/test-only EC CA/leaf key 不涉及设备配对，不可用于实际部署。

相关 manager **39 PASS、1 explicit browser ignored**、upstream security **6 PASS**、
strict CLI Clippy、17 tools PASS；S02 actual Chromium regression **1 PASS（2×16）**。
完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` PASS，`/tmp/ios-s03-federation-full.log`：
Linux x86_64 / Rust 1.97.1 / MSRV 1.80.1 / Python 3.12.13，Rust **1651 PASS、0 FAIL、
2 ignored**（既有 PBKDF2；browser另显式PASS）、Python **8 PASS**，66 feature builds/
四组、7+7 doctests/三个 snippets、C ABI/19-byte capture、wheel/sdist rebuild/install/
import、17 tools、118 inventory/实际help、**38** original+masked CLI/service captures、
actionlint、diff/staged check PASS。新增坏 upstream 私密配置最大verbosity capture：
启动无 listener/discovery、原路径值不泄漏。CLI SHA-256
`fc718d5b92f2a25f21ed1a5485edc5cf90d3d49a575b112bbd192df360ea27f9`。

16 个相关实现/合同文件已提交、工作区干净；本记录/证据/质量作独立文档收尾。
S03 Linux host 可执行范围已交付；macOS/Windows private-file/ACL/TLS实测、实际
部署 host-key/certificate/forward 接入及真实设备 topology/EOF 仍待条件，不能把
owned fixtures 作为这些 PASS。第二类/目标保持进行中，随后推进 **Q02 regression
matrix/runner 与 FUP-01–04 既有内容核查**；S01/S02/F03/F05 待条件不被删除。
没有配对、DDI、真实 TSS、刷机/擦除、推送/发布、历史改写或参考 checkout 修改。

### 大类 2 进度与主机出口：Q02 回归证据、owned cleanup 与 FUP 复核（2026-10-01）

实现提交：`e9cc536351d08d2dc1fa3504d69259b9f215e39b`；
[验收证据](validation/regression-runner-2026-10-01.md)、[运行/安全合同](regression.md)、
[风险矩阵](regression-matrix.json)、[FUP 当前复核](real-device-followup-tasks-2026-08-30.md)。

- 新增标准库主机 runner，27 行矩阵分开登记 5 host 和 22 real-device 条件。
  USB/Wi-Fi、传统/CoreDevice、RSD/remoted、private MAC/altIRK、Cryptex、Display/
  HID、富剪贴板、app/file/XCTest 及 Windows T-05 九子例/UNC、InstallCoordination
  wire 观察均保留；当前执行 5 PASS、0 FAIL、22 NOT_RUN，不填造设备/iOS/state。
- 统一直接 subprocess 参数、实际 exit、duration、contract、raw/independent masked
  hits、timeout/overflow/incomplete、owned PID/group 与 cleanup；child sys.argv 在 RAM
  比较 `/`/`.`/missing/Unicode/private synthetic path，其余标明是 OS 输入而非独立
  CLI 解析观察。公开参数 allowlist，私密值只有索引/长度/本轮 keyed tag，key 不入证据。
- HEAD/dirty、binary/version、matrix/runner/capture SHA-256 和 UTC/host/runtime
  provenance；raw 输出与异常 text 不打印/归档。CLI 参数错误也不回显未知值；
  未允许 case 在 command spawn 前拒绝。非零 exit 需要精确阶段合同，不能当空成功。
- 每轮新建 checkout 外私密 identity/key 与独立 evidence；仅 `report.json` 可交付。
  注册 root/file identities 与绝对边界后只删本轮 regular files；unknown、symlink、
  replacement 保留并失败，无递归删除。Linux capture 正常退出也检查/终止尚运行
  的 owned group，覆盖已关闭 pipe 的 descendant；不终止其他 session。非 detach
  fixture 限定、其他 Unix 保守结果及 Windows process-tree/ACL 未验收写明。
- 17 个新增工具用例观察真实 child argv、原始秘密/失败/超时/agent SIGTERM 收尾、
  contract exception、unknown artifact、symlink/regular replacement、case 前置拒绝、
  CLI parse 和异常 cleanup。FUP-02 文本边界核查完成；FUP-01 公共排障已在现有文档，
  本地两份 `handoff/` 报告不可访问，勘误不能补造。FUP-03 Linux 新工具主机范围交付，
  不声称修改了不可访问的仓库外旧工具；FUP-04 全部保留 NOT_RUN。
- `validate.py regression` 接入 full 与 Linux CI check（发布已依赖该 job）；只读
  主机范围，不增加任意命令 manifest、设备 executor、配对或状态变更。

先执行相关工具测试和 runner：`python3 -m unittest discover -s scripts -p 'test_*.py'`
**34 PASS**，`python3 scripts/validate.py regression` **5 PASS/22 NOT_RUN**；
CPython **3.9.25 与 3.13.15** 分别执行新增 17 tests 和实际 runner，各 PASS。
审查后补强参数错误/case 前置校验，再完整运行 `python3 scripts/validate.py full
--python /home/oslo/projects/rust-ios-device/.venv/bin/python` PASS，日志
`/tmp/ios-q02-regression-full-final.log`：Rust **1651 PASS、0 FAIL、2 ignored**
（既有 PBKDF2；browser 前项显式 PASS，此 Python 改动未重跑浏览器），Python
**8 PASS**；66 feature builds/四组、7+7 doctests/三个 snippets、C ABI/19-byte capture、
wheel/sdist rebuild/install/import、**34** tools、118 inventory/实际 help、**38** raw
CLI/service captures 与新 host regression 均 PASS；actionlint、Python 3.9 syntax、
diff/staged check PASS。Binary SHA-256 仍为
`fc718d5b92f2a25f21ed1a5485edc5cf90d3d49a575b112bbd192df360ea27f9`。

实现仅 10 个相关文件，提交后工作区干净；clean HEAD `e9cc536` 实际重跑 host runner
PASS，证据 `/tmp/ios-regression-evidence-eur6rg8k/report.json`：dirty=false、5 PASS/
22 NOT_RUN，删除三个本轮私密文件，unknown/unsafe/agent failures 均零，目录0700/
report0600、只有 allowlisted report。证据在 checkout 外，不加入 Git。

| 第二类任务 | 当前出口与真实剩余条件 | 状态 |
| --- | --- | --- |
| Q01 | 118 项权威清单和证据检查；没有因此认证稳定设备 workflow | 完成 |
| Q02 | 主机捕获/清理/矩阵/FUP 文档范围交付；缺精确 selected device/state/service/fixture/scope、device adapters 实测、Windows process tree/ACL/输出/UNC 与历史报告勘误 | 待条件 |
| Q03 | Linux 输出失败/原子文件主机合同交付；macOS aliases、Windows/UNC、实际 AFC 输出待对应条件 | 待验收 |
| S01 | 原生 listeners 边界、manager/CLI/doctor/debug scopes/reload 与独立 upstream 凭据主机交付；平台文件权限和真实转发/撤销/Apple cleanup 待验收 | 待验收 |
| S02 | 既有入口 Host/Origin/方法/CORS/短票据和实际 Chromium host fixture 交付；F05 新入口、其他平台/浏览器及真实设备流程待条件 | 待验收 |
| S03 | HTTPS/WSS verifier、独立 token/CA、redirect/两跳 host fixture 交付；平台文件/TLS、实际 SSH/TLS forward deployment 和真实 topology/EOF 待条件 | 待验收 |
| S04 | 自有错误/日志/Debug 与 CLI dependency 来源门禁验收；未来新增接口继续沿用，显式设备数据不自动改写 | 完成 |

第二类当前可执行 Linux 主机范围完成验收、实现与文档收尾，不宣称整类真机/平台
完成。D01/D02 同步记录；全部待条件子项保留，目标仍 active。接续第三类 **A01
兼容合同、R01–R03 模块边界，再 R04/A02**：依赖的是已就绪 Linux 构建/路径/安全
接口，不把未有的 Windows/真机证据当作已就绪。没有设备操作、推送、发布、参考
checkout 修改或历史改写。

### 大类 3 进度：A01 公共兼容合同与真实下游检查（2026-10-01）

聚焦修复提交：`8d07bc977a8c8510085e783a3f6b7cb39ec83c5b`；
合同/fixture 实现提交：`076e9097afe763456f0b063edd96d1b4bc9e2397`。
[候选接口与迁移规则](public-contract.md)、[验收证据](validation/public-contract-2026-10-01.md)。

- 定义 Rust root/public module、CLI 参数/JSON、C ABI 与 Python factory/metadata/
  patch 候选 profiles，和 patch/minor、废弃/alias/versioned replacement、发布版本
  一致性规则。候选合同与 Q01 的实验/真实设备成熟度分开，未认证 stable workflow。
  现有 public struct literals、exhaustive enums 加字段/分支或 retroactive
  `non_exhaustive` 的破坏性明确登记，不为重构修改其现有封装。新类型从初版使用
  私有字段/constructor/builder、可扩展枚举等约定，A02优先 additive分类方法。
- 新增外部 integration consumer：真实 root imports、现有字段literal/exhaustive
  `CoreError`/`TunMode`/`DeviceEvent`、metadata absent vs unadvertised、codec roundtrip、
  未poll async signatures 与 service alias typeidentity；默认4/别名5/full5 tests。
  没有 poll 设备future、扩大private模块可见性或新增设备操作。
- 四项实际 CLI 经 owned fake usbmux 验 USB/Network 两设备与 empty 列表的 JSON/
  human合同、无混入stderr、准确exit/raw scan；request仅ListDevices，原始数据RAM，
  bounded frames/socket/capture、只输出安全计数。接入 full 与 Linux CI check。
- 此fixture复现 `ProductID` 被PascalCase当作 `ProductId` 的已有解码问题，原
  `product_id`总落为0。只读 go-ios 类型确认后独立修复 canonical key+legacyalias，
  listing/attach/missing两项unit回归。JSON字段/type和missing0不变，实际值修复
  有迁移说明；没有把源码fixture当作真实usb设备观测。
- C smoke对14个公开函数进行原有 typed pointer赋值/link，固定status0–8、mode
  和layout，继续null/UTF8/outputreset/free回归；不改ABI。Python installed smoke
  验factory参数名/order/default、list无参、公开patch/metadata和禁止直接构造。
  实测发现restore_all为内部方法，未为测试新增导出；合同按实际install/restore。
  说明C非NULLclose后pointer不能重复使用、Python安装patch可延后释放、无Apple ACK。
- 中英文根README、四crateREADME、architecture/build/contribution/PRtemplate与
  CHANGELOG同步。既有0.1.13不重发；实际unused一致版本仍属D02后续，不能自动发布。

相关检查：默认consumer **4 PASS**；`cargo test -p ios-core --locked
--no-default-features --features icon_service,screen_capture --test public_contract`
**5 PASS**；mux protocol filter **4 PASS**（其中两个新增）、strict core Clippy PASS。
`validate.py ffi`/`python-package`/`tooling-tests`/`compatibility`均PASS；Python3.12
wheel/sdist构建安装新signature smoke通过，独立candidate wheel实际安装到
**3.9.25/3.13.15**并执行同一smoke，均PASS。第一轮Python检查因误把内部restore_all
列为导出而FAIL，纠正测试/公开合同后重新构建安装PASS，不隐瞒此迭代。

随后完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` PASS，日志
`/tmp/ios-a01-contract-full.log`：Rust **1658 PASS、0 FAIL、2 ignored**（既有PBKDF2/
显式browser gate，browser此项未重跑），Python **8 PASS**；66feature/四组、7+7doctests/
三个snippets、C ABI/19byte capture、wheel/sdist rebuild/install/import、34tools、118
inventory/实际help、38 raw CLI/service captures、Q02五host/22NOT_RUN、新CLI四项
compatibility均PASS；actionlint、diff/staged和local links PASS。Binary SHA-256：
`f488aab365eb475ce915cef85d1ea9b35358269908482e68a2e0edd9043cd5c1`。

1个协议修复文件先提交，18个相关合同/fixture/doc文件随后聚焦提交；工作区干净，
clean `076e909` 再跑CLI compatibility四项PASS。本记录/证据/质量另行文档收尾。
A01的规则和代表性主机验收完成，不替代A02错误分类、L01取消/期限、A03 SDK资源
扩展或D02跨平台/真机1.0准备。ProductID真机仍无新观测，B03/B04/Q03平台条件
继续保留。第三类保持进行中，接着 **R03普通模块与职责边界**，再完成R01/R02及
R04/A02；依赖所需Linux接口/构建/候选合同已就绪。目标active，没有真机/配对/
DDI/TSS/刷机擦除、推送发布、参考checkout修改或历史改写。

### 大类 3 进度：R03 device 普通模块与明确边界（2026-10-01）

实现提交：`f9b96f927431d1e099e121c7347d4a2df7dd5c69`，17 个相关文件；
[结构说明](architecture.md)、[验收证据](validation/device-boundaries-2026-10-01.md)。

- 移除 device 全部 `include!`，公共类型继续定义在 facade，四个连接入口和配对
  发现函数明确 reexport；crate-root 与 `ios_core::device::*` 路径均保持可用。
  root 不再供给连接实现隐式 imports、协议常量或辅助函数。
- `connection` 只选择 USB/TCP/direct RSD/RemotePairing 策略；`lockdown_transport`
  管 pair record、拨号、session/service 与 plain/TLS CoreDeviceProxy stream。
  将原在 RSD 文件中的代理包装归回 lockdown，发现函数归到 `discovery_match`。
- `service_routing` 用 RSD directory/请求名返回实际 descriptor 的 name/port/features，
  或在传入 stream 上执行有界 check-in；`rsd_connect` 只负责 optional handshake。
  隧道 activation 使用既有 `TunnelConnection`/authenticated stream/TunMode 输入，
  endpoint 管 direct/userspace socket；RemotePairing 的 credentials/pairing/codecs
  留在策略内，沿用窄可见性。时间预算、重试、字段与协议分支未改。
- 将测试放到职责所有者，未为旧测试扩大 nested helper 到 crate/public 可见性。
  公共 consumer 追加所有连接 root/device 路径与 mdns 发现签名，future 不 poll。
  新 TCP fixture 只用 owned loopback listener，确认 pair-record flag 的两种取值
  都保持双向字节流、不进入 usbmux 配对记录读取。
- diff 审查：原 47 个 device 测试按函数名全部保留且 PASS；移位函数体比对在
  whitespace 与同值发现 timeout 常量引用改名归一后保留 174 个函数体，新增的
  只有 TCP fixture 和一个分组测试临时目录 helper 副本。此比对是源码审查辅助，
  不代替执行测试或真实设备证据。架构/CHANGELOG 同步。

相关检查：`cargo test -p ios-core --all-features --locked device::` **58 PASS**，
其中 device **48**、substring filter 同时匹配已有 coredevice **10**；重构前分别
47/10。`--no-default-features` device **17 PASS**；再加 `--features tunnel` **37 PASS**；
外部 public_contract 默认 **4**/full **5 PASS**。默认 unused metadata import 与
tunnel-only unused strategy import 在迭代中修正；最小 feature 中已有 dead-code
警告保存在 feature logs，未宣称所有最小组合严格 Clippy 零警告。

随后完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` PASS，日志
`/tmp/ios-r03-modules-full.log`：Rust **1659 PASS、0 FAIL、2 ignored**，Python **8 PASS**；
66 feature/四组、7+7doctests/三个 snippets、MSRV/strict full Clippy、C ABI/19byte
raw capture、Python3.12 wheel/sdist 重建安装、34 tools、118 inventory/实际help、
38 raw captures、Q02五host/22NOT_RUN、四CLI list contracts 均PASS。二项 ignored
仍为 PBKDF2/显式 browser probe，未在本项重跑；A01 的3.9/3.13与S02/S03浏览器
证据保留为原提交证据。Binary SHA-256：
`53e4262f881accee53e2b9d893d5108a59d2cbd554242eb758c4655954068953`。

相关源码/文档聚焦提交，提交后工作区干净，再做本记录/证据/质量/索引收尾。
R03 以计划明确的普通模块、既有主机路径回归、公共兼容和最小 feature 条件完成；
没有以 IPv6 endpoint 选择代替真实 kernel TUN 创建，或以合成 check-in/TLV 代替
Apple 配对/服务生命周期。真实平台/设备条件继续由 B03/Q02/S01/L01 等对应任务
保持待条件/待验收。第三类继续 **R01 备份协议/路径/过滤/会话与清理边界**，随后
R02、R04/A02；整体目标 active，第三类尚未全部完成。

### 大类 3：R01 备份边界与输出验收完成（2026-10-01）

基于 `162fd23d796cdb92b2993b081c5a5cf87cf70710` 开始 R01。实现提交
`a3336572bca72e1cfd083afde075df6182f36c76`（17个相关文件），完成下面记录的主机
边界与回归验收；各轮失败与修正保留，早期“仍在验证”描述为当时状态。
已将 bounded presets/regex/manifest/receive-time 策略移入 `filter`，保留原 public
reexports；三个过滤测试随职责移位，第一轮 backup2 filter 回归 **65 unit PASS、
1 integration PASS、1 ignored**。这条 substring filter 未覆盖 integration 文件
全部11项交互；完整交互在 R03 full 的未改 backup2 基线已 PASS，R01 正补独立
`--test backup2` 检查。CLI baseline `cmd::backup::` **18 PASS**。

`protocol` 首轮移入请求 shape、有界 frame 与 numeric/status/redaction helpers，
仅消费传入 stream/value。可见性生成初轮误命中非字段，fmt FAIL 后已限定范围，
重跑后 **65 unit PASS、1 integration PASS、1 ignored**；显式 `--test backup2`
**11 PASS**，覆盖完整 backup/restore/list/password 等合成协议交互。清理无用测试
import 后继续拆分。协议字段/预算未有意改变，尚未宣称整个重构或验收完成。
继续将 bounded transfer accounting/operation deadline 和客户端会话方法移入
`session`，显式列出 protocol、filter 和宿主 helper 输入。为避免扩大私有 method
可见性，旧 unit tests 暂留在该 owner，路径 tests 后续随 host owner 整理；补齐
ErrorKind/Apple epoch 与条件 test imports 后，**65 unit PASS、1 integration PASS、
1 ignored**，显式11项完整交互仍 PASS。没有新增公共字段或扩大 pairing/device 操作。
已拆 `host_fs` 的 layout/path/symlink/staging/metadata 与 typed blocking runner，
将21项 host tests 归到该 owner、options Debug 归回 facade；manifest/会话明确
引用宿主接口。移位初轮漏改7处 manifest qualified root helper 调用，编译失败后
改为实际 owner，重跑 **65 unit PASS、1 integration PASS、1 ignored**，完整11项
交互仍 PASS；清除未使用的 host fixture helper。此前 default（无 manifest）
unit/11项交互 PASS；
最初误用不存在的 `backup2` feature 未执行测试，已按实际默认公开模块纠正。
移位遗留 doc comments 与 test 后 item 位置修正后，core strict Clippy PASS。
CLI 36处 println 改为 fallible writer，部分内容与 newline 写失败明确返回IO error；
CLI **20 PASS**（原18+两个新输出失败回归）。Local unback/extract/list、create
后处理与 restore 的 metadata/applications preflight 已改 owned blocking worker，
password 持有 Zeroizing；worker 返回原错误链，CLI **20 PASS**。另加入明确 layout
ownership 的 host worker 接口，将 create-directory、upload staging 和 download
resolve/open 从 async worker 移到阻塞线程；正跑取消/字节流/安全回归。
后续已将 move/copy/remove/list、upload commit 的路径检查及完整磁盘步骤放入
typed layout worker；会话只在 worker 完成后跟踪 placeholder 与发送 status。
JoinError 保留实际 source 而非转换字符串。新增 single-thread timer/typed source
回归与真实 filtered backup 的 upload→move/copy/missing-source/list/remove→disconnect/
cleanup 回归。后者先 FAIL：既有 exact-file relocation/copy 拼接 empty suffix 产生
尾部分隔符，Linux cleanup 返回 NotADirectory；已修正为确切 destination，重跑
**68 unit PASS、1 integration PASS、1 ignored**。新测试初轮 plist Dictionary 的
`.into()` 类型推断 FAIL 已显式约束后修正。默认无 manifest **40 unit PASS、1
integration PASS**；显式 integration **11 PASS**，下游 facade **full 6/default 5 PASS**；
CLI **21 PASS**，core/CLI full strict Clippy PASS。
真实 CLI fixture 首轮使用错误 flag/遗漏 ProductVersion，随后 archive integer 字段
未按现有 UID object table 编码，均 FAIL；按实际 JSON 默认/`--no-json`、metadata 与
解码器纠正后，**3个 actual-process contracts PASS**（非空 human 49B、JSON 199B、
closed-pipe 70B），原始 capture raw_hits=0、owned process/fixture cleanup=true；closed
child exit1、IO BrokenPipe、无 panic。新增4个工具失败路径 PASS，未知/替换文件及
目录保留，missing binary 不回显路径并失败；scope 已加入 Linux CI/full。
输出失败101→1和 partial output/已执行副作用、restore local preflight 提前的变化已
写入 `docs/backup-contract.md`。已有7类 public helper、所有 facade fields、9个 wire
request shape、资源预算与11项旧 integration 交互保留；正在跑 §11 full gate。
最终 §11 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，日志
`/tmp/ios-r01-full.log`：**1666 Rust、8 Python、0 FAIL、2既有 ignored**；fmt/check/
strict Clippy/MSRV、66 feature/4分组、doctest 7+7/3 snippets、C ABI/19B原始capture、
Python3.12 wheel/sdist重建安装、38 tooling、118实际help、38 raw诊断、Q02 5 host PASS/
22 real-device NOT_RUN、4 CLI list和3 backup actual-process contracts均通过。
最终审查将 usbmux 固定 sentinel 改为新建 owned loopback trap；之后再次执行38项
tooling及3项 backup contracts **PASS，device_connections=0**，Python3.9/3.13的新
工具失败路径各4 PASS。没有新增这些版本的 wheel 证据或重跑历史 browser gate。
CLI仍0.1.13，受测二进制SHA256
`8a4fd63fca0e1cc7e1f9ff617a1ec338c55bacff28ef4d69e78fd45dabd7d60f`。

diff/暂存空白、actionlint及变更文档链接PASS；42个原 facade unit test名字全部保留，
9个 request字段shape一致，150个原函数body中141个归一空白后相同；9个变化逐项审查为
host/session worker、JoinError原source和exact-file tracking修正。此辅助不是AST证明。
`docs/validation/backup-boundaries-2026-10-01.md`记录命令、结果、代码哈希及边界，
architecture/build/output/CHANGELOG持续更新；未修改参考checkout或混入无关变更。
R01主机模块/既有安全预算/backup-restore-filter-manifest-cancel-output验收完成。
Windows/macOS native路径、真机backup/restore、started blocking job无法由future drop
停止及更强deadline/cleanup ACK，仍按B03/Q02/Q03/L01保留真实条件，不由主机模拟替代。
第三类继续 **R02 WebInspector职责与注入边界**，随后R04/A02；未推进第四类，整体目标
active，全部31项尚未完成。收尾提交仅更新计划/质量与证据索引，不推送或发布。

### 大类 3 进行中：R02 WebInspector 边界（2026-10-01）

R01 文档收尾提交 `e069a46cdcd0f207c22d01cf55f2155f0c7caf5c` 后工作区 clean，
仍在 `feat/device-service-updates-20260927`；R02 当前尚未实现提交。所需 Linux
构建、core stream/session 和 S02 admission/lease 接口已就绪；其他平台、真机/CDP
客户端证据仍独立保留，未降低这些条件。基线 `cargo test -p ios-core --all-features
--locked webinspector` **20 unit PASS、1 filtered integration PASS**，未当成完整
integration 文件；R01全量未改CLI中 `cmd::webinspector::` **28 PASS**。

已将 bounded plist frame/typed decoding 移入 `wire`、monotonic deadlines 移入
`timing`，5个既有测试随 owner 移位，**20 unit/1 filtered integration PASS**。
`model` 保留公开 metadata/event 类型与 facade reexports；漏保留仍在 By 使用的
Serialize import 初轮编译 FAIL，补齐后仍 **20+1 PASS**。随后将 InspectorSession
及 Target envelope/response 状态移入 `inspector`，将 AutomationSession/By/JS 与
元素语义移入 `automation`；三个测试移入 owner，**20+1 PASS**，多余 test import
在后续清理。客户端 stream/cache/event routing 及4个测试移入 `client`，只将跨
session 调用的两个私有方法设为 pub(super)，字段保留私有，facade 保留原公开路径。
首次漏加生产使用的 WirType import 编译 FAIL，已补齐；最终 all-features 以及
`--no-default-features --features webinspector` 均 **20 unit + 1 filtered integration
PASS**，未改 CLI 的专项基线 **28 PASS**，core all-target/all-feature strict Clippy
**PASS**。六个 owner 的111个原函数 body 归一空白后均与原 facade 一致，原函数名
无遗漏；这是审查辅助，非AST证明。CLI开始将真实 device/service 连接策略与总超时
移入 `connection`，明确返回 device guard、boxed service stream 和uses_rsd；CLI
专项仍 **28 PASS**。该连接模块尚没有注入 factory，未宣称F03接口已就绪。
没有新增或猜测 Apple selector、修改既有 JS 字节、权限策略或网络端点。

随后将CLI的发现/选择/结果解释、shell输入、一次性命令执行和脱敏诊断分别移入
`workflows`、`shell`、`tasks`、`diagnostics`，原四个测试随owner移动。首次漏掉
shell模块声明及跨owner imports 编译FAIL，修正后 **28 PASS**。继续将命令grammar/
dispatch、HTTP startup/共享state、CDP、WebDriver移入普通私有模块，各owner显式
imports；原测试移入owner，BrowserPageSession Drop测试移入CDP而未扩大其字段
可见性。生成拆分补丁时两次前置断言/映射失败均未应用，超过输出预算的完整补丁
未应用，改为逐文件完整补丁。首次应用误给HTTP参数添加字段可见性、漏移Debug
derive/imports，编译FAIL；修正后原 **28 PASS**。

内部 `InspectorSource` 返回owned boxed stream和可选device guard，生产仅安装
真实DeviceInspectorSource，四个桥接连接位置均在admission后调用；无CLI/HTTP
mock开关。新增实际保护router/listener的合成plist测试和外部consumer，夹具初轮
Dictionary泛型类型推断及Drop类型struct-update编译FAIL，后续修正中。未新增Apple
selector或改变真实协议行为；夹具Runtime.syntheticFailure是测试专用错误请求。

夹具修正后执行测试为 **29 PASS/2 FAIL**：Runtime已回复，但fake teardown selector
与core现有 `_rpc_forwardDidClose:` 不符；修正夹具（未改真实协议）后专项 **31 PASS**。
三个新增测试通过实际protected Axum listener/router和owned duplex plist peer，覆盖
CDP/WebDriver未授权零source调用、已授权精确合成device selector传递和typed错误脱敏；CDP页面的
发现/不暂停attach/Target嵌套Runtime回复与错误/显式teardown，以及browser flattened
session/Runtime/sessionId保持/detach/teardown。派生计数与owned任务join确认，未将
模拟算作真机或外部CDP客户端证据。CLI strict Clippy初轮生产构建发现test-only import
及遗留doc comment owner，修正后 **PASS**。

全feature公开consumer **7 PASS**、整个WebInspector integration **12 PASS**；最小
`--no-default-features --features webinspector` 同两test target **6+12 PASS**。一次使用
错误 `--test services` 命令直接FAIL且未跑测试，随后按实际target名改为webinspector。
111个core原函数body归一空白后全部保留，CLI118个函数无遗漏，108个body一致、10个
变动逐项审查为内部source/state/module调用；非AST证明。最初比较把结果struct和
其他模块test routes混进baseline，修正边界后grammar、24个生产route paths、OpenedTabRow/
LaunchResult字段均保留。没有公开字段/枚举变更，生产实际device guard保持原scope。

以上为门禁前进行中记录；后续结果如下，不以历史中间状态作为当前结果。

### 大类 3：R02 主机验收与收尾（2026-10-01）

| 字段 | 结果 |
| --- | --- |
| 任务/实现提交 | R02所有模块/职责/注入及主机兼容验收；`07ec8977fd5e8e421982348bcc79e376909d698e`，23个明确相关文件，6162 insertions/5260 deletions；分支保持 `feat/device-service-updates-20260927` |
| 实际行为 | core六个private owner和CLI十个owner/fixture分离；public facade/模型/error/常量保留；真实连接guard保持，注入接口只允许Rust内部安装owned服务；生产权限/参数/24个路由/结果字段及JS/wire语义保留 |
| 相关检查 | CLI31 PASS；core unit20 PASS+1 filtered integration；完整WebInspector integration12 PASS；public consumer全feature7/minimal6 PASS；strict core/CLI Clippy PASS；历史FAIL及修正见前述记录 |
| 主线命令/结果 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`，`/tmp/ios-r02-full.log`，第11节全部PASS：Rust1670/Python8 PASS、0 FAIL、2既有ignored（PBKDF2和独立S02 browser gate）；66最小feature/4组、default/full doctests7+7/3 snippets、MSRV1.80.1、C ABI/19B raw capture、3.12.13 wheel/sdist重建安装导入、38 tooling、118 inventory/实际help、38 raw、5 host/22 device NOT_RUN、4 CLI list和3 backup contracts全部PASS |
| 受测binary | `ios`0.1.13，SHA-256 `b97a99409fe6a4d9fa598cb9d3064bbd46d3d3effb8f31b1644d515c216845c2`；raw hits/masked hits均0，owned cleanup成功 |
| 证据/状态 | [验收索引](validation/webinspector-boundaries-2026-10-01.md)；R02完成其明确的主机module/contract/injection验收；派生协议计数与实际Rust WS交互保留，未升级能力成熟度 |
| 真实剩余条件 | F03真实Playwright/Puppeteer通过Rust桥和F04各domain证据仍待任务实施；真机Safari/WebView/JSContext/automation/版本路由与设备期限/清理，以及平台执行各自待条件；R04输出失败、A02错误分类、L01生命周期预算未被R02替代 |

最终diff及staged whitespace审查PASS，55个相关本地文档链接PASS；实现提交内容
已核对，只留下本类文档收尾变更（计划、验收索引、能力证据、质量回顾）。早期
3.9/3.13构建及S02独立浏览器证据仍为历史结果，此轮未重跑。第三类继续 **R04登记
既有plist/JSON编码与输出合同，再A02错误分类**。总体目标active，未进入第四类；
无推送、发布、参考checkout修改或实际设备操作。

### 大类 3 进行中：R04 编码/输出合同基线（2026-10-01）

R02实现 `07ec8977fd5e8e421982348bcc79e376909d698e` 和文档收尾
`612b480ef769575026cdbd49faf15faf5fd8f67d` 均已核对，收尾后工作区clean，分支
保持不变；R04当前未提交。按要求先登记历史编码，再抽共享实现。普通转换的Data
数组/hex、Date/Uid支持与legacy null、MDM tagged base64/独立redaction、i64/u64/
非有限float、future unknown null及不可构造integer fallback已记录于
[编码合同](plist-json-contract.md)。没有修改真实协议、输出或编码策略。

为三个crate增加本crate内的固定fixture（不引入跨包源码引用），14个原转换owner
各调用旧实现，另3个原生plist serde测试覆盖值与pretty结果。fixture包括Data/空Data、
Date、Uid/u64 MAX、i64 MIN、有限/非有限real、字符串/bool及嵌套容器，序列化再解析
检查固定值。`cargo test --workspace --exclude ios-py --all-features --locked plist_json_`
**17 PASS、0 FAIL/ignored**（CLI11/core4/FFI2），日志 `/tmp/ios-r04-legacy-baseline.log`；
filter后integration无selected cases，不计为成功integration。当前仅增加baseline测试
和记录，生产转换函数尚未抽取。

随后新增公开 `plist_json` 私有字段policy与非穷尽DataEncoding（无默认隐式格式），
支持NumericArray/HexString/Base64Object、历史Date/Uid null及不可构造整数zero
fallback。借用/owned入口使用迭代容器walk，owned字符串/键移动；不引入transport、
自动脱敏或静默截断，现有输入/输出预算由调用层保留。4096层合成值walk测试采用
迭代解包/销毁，没有把递归serialization/drop算作通过。第一轮误用plist Values反向
迭代器，编译FAIL；改为work区间反转后最小core专项PASS（既有最小feature warnings
保留，未当成严格Clippy）。13个原转换owner变为显式policy delegate，MDM只共享
普通scalar编码，敏感键/数组/dict traversal保留。包含原17基线与3shared测试的
`cargo test --workspace --exclude ios-py --all-features --locked plist_json` **20 PASS**，
`/tmp/ios-r04-shared-baselines.log`；MDM包括秘密键旧回归 **7 PASS**，公开consumer最小
feature **6 PASS**（新增1个policy consumer），分别为 `/tmp/ios-r04-mdm.log`、
`/tmp/ios-r04-public-minimal.log`。未更改C ABI header或现有公开类型。

Backup fallible writer及两个原失败测试迁移到CLI共享 `output` owner，backup保留
private reexport；无newline写入显式flush。新增内部outln/out宏通过`?`保留原IO链。
语法扫描避开Rust字符串/raw literal、comments及测试模块，生成61个明确文件补丁，
603个production stdout宏调用已应用完整逐文件补丁；该次迁移的相关compile检查
正在运行，未宣称helper/callback已接受或R04已完成。中断前3个测试handle已确认
terminal exit0，未重启已结束进程。

迁移初检 `/tmp/ios-r04-output-migration.log` **FAIL84**：28个void human helpers不能
传播`?`及一个unused import；已修正为Result并在57处调用传播，未丢弃结果。
`/tmp/ios-r04-output-helpers.log` all-target/all-feature check PASS，随后严格CLI Clippy
PASS（`/tmp/ios-r04-output-lint.log`）。6处XML stdout共享原serializer/extra newline并
显式flush；btlogger补最终flush，file-relay原已有检查；raw stdout审查后CLI严格Clippy
仍PASS（`/tmp/ios-r04-output-raw-lint.log`）。生产stdout println/print已全迁移，剩余
两处println为gated browser test probes。现有stderr进度/异步callback合同留A02审查，
不能把stdout检查扩大为全部diagnostics或stream生命周期已完成。

实际进程复现旧R02二进制help/version在关闭stdout时exit0；新main检查clap print的
IO结果，返回exit1，既有失败的stderr报告使用fallible best-effort以保留exit1/2。
扩展owned ListDevices假服务CLI fixture（仅一条list请求，无设备/service连接）：
原JSON/human空/双设备4项，加JSON/human closed stdout及help/version closed stdout
4项。旧R02二进制 **4 PASS/4 FAIL**（`/tmp/ios-r04-old-binary-output-contract.log`）；
构建新CLI后 **8 PASS**（`/tmp/ios-r04-output-contract.log`），三项工具负例PASS
（`/tmp/ios-r04-output-tooling.log`），CLI全量 **618 PASS、1既有browser ignored**
（`/tmp/ios-r04-cli-all.log`）。这是实际主机进程证据，不是设备验证。

shared converter再改为逐层cursor，避免宽容器额外暂存所有siblings；不宣称测得性能
改善。`/tmp/ios-r04-cursor-baselines.log` **20 PASS**，core/CLI/FFI严格Clippy PASS
（`/tmp/ios-r04-shared-output-lint.log`）。已确认旧测试/Clippy/CLI handles terminal
exit0。随后追加owned 4096层及XML固定bytes/partial IO source回归，当前专项待重跑。

最终XML专项 **4 PASS**（`/tmp/ios-r04-xml-writer.log`），固定XML bytes及partial IO
source保留；owned/borrowed深度4096的最终core转换filter **7 PASS**（无selected
integration不计入），`/tmp/ios-r04-final-converter.log`。显式public consumer/AX
integration **8+7 PASS**（`/tmp/ios-r04-public-service-contract.log`），工具三负例在
Python3.9.25/3.13.15各 **3 PASS**（`/tmp/ios-r04-tooling-py39.log`、
`/tmp/ios-r04-tooling-py313.log`）；不是新增wheel runtime认证。
对HEAD与worktree作保护literal/comment的词法审查：61文件603个stdout宏的全部
argument tokens与line/formatted种类相同（`/tmp/ios-r04-output-diff-review.json`），
不解释为AST等价或全部命令真机PASS。changed local Markdown links **129 PASS**。
完整 `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`
已启动（`/tmp/ios-r04-full.log`），当前Rust/Python、Clippy、MSRV通过，feature矩阵
进行中，完整gate尚未接受。

剩余：完整第11节主线结果、最终diff审查、实现提交/准确哈希与文档收尾。成功
JSON/Data/Date/Uid策略不变；普通
结果stdout失败从panic101变为保留IO链的exit1，help/version失败从exit0变为exit1，
均须进入迁移说明。R04/第三类进行中，总体目标active；A02随后推进。

### 大类 3：R04 主机验收与收尾（2026-10-01）

实现提交 **`c365d5569cb94cc9bade2d17499a37040ae9736a`**，85个明确相关文件；提交后
仅本计划留在工作区，未混入AGENTS/参考checkout/无关变更。实现已完成历史策略
共享与stdout错误传播；[验收索引](validation/plist-output-2026-10-01.md)、
[迁移合同](plist-json-contract.md)、中英文README、crateREADME、architecture/
public-contract/usage与CHANGELOG同步。Q01新增单项host evidence，不升级成熟度。

| 字段 | 实际记录 |
| --- | --- |
| 状态 | R04完成：先登记17旧编码fixtures，再共享策略，保留13 delegates与MDM独立redaction；core/CLI/FFI成功fixture及明确stdout失败路径已验收；第三类尚待A02 |
| 主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` PASS；`/tmp/ios-r04-full.log`，所有运行handle已确认terminal exit0 |
| 分拆结果 | 1693 Rust PASS、8 Python PASS、0 FAIL、2既有ignored（PBKDF2与独立browser gate）；66 features/4 groups、7+7 doctests/3真实下游snippets、严格Clippy/MSRV、C ABI及19B raw capture、3.12 wheel/sdist重建安装PASS |
| 工具与进程 | 41 tooling PASS、118实际help inventory、38原始diagnostics、5 host regression/22 device NOT_RUN；8实际CLI输出合同与3本地backup合同PASS，owned cleanup完成、无原始secret hits/设备dial |
| 差异审查 | 61文件603个result宏kind/arguments词法保留；28个私有helper与57调用传播Result、6 XML sites/BT flush/core策略/main clap IO边界已审查；不是AST等价或真机流程证据 |
| 受测二进制 | ios0.1.13，SHA-256 `d6e9bc8e8f86fad2578c5e3508bd1b26e39cb77c577ba328904faacdb5e84d73` |
| 迁移 | 普通stdout失败panic101→IO error exit1；help/version失败丢弃error exit0→exit1；已失败命令的stderr报告失败保留exit1/2；成功bytes/JSON/API/C ABI不变，输出失败不意味着副作用回滚或可重试 |
| 真实剩余条件 | A02稳定分类/各入口映射、普通stderr callback/JSON混入/stream/partial语义仍待实施；L01输出失败后的有序设备清理与期限未认证；macOS/Windows及真机服务workflow各自待条件；未来unknown variant/legacy整数fallback不可构造，仅登记和审查 |

此次完整gate没有重跑S02独立浏览器或A01 Python3.9/3.13安装wheel；R04工具负例
在3.9/3.13各3 PASS另行记录。没有配对、DDI、真实TSS、恢复/擦除、推送、发布或
历史改写。文档收尾核对后自动继续 **A02错误与机器输出合同**；R04接口/主机结果
已就绪，因此A02依赖解除，第三类/总体目标仍active，尚未进入第四类。

### 大类 3 进行中：A02 类型分类与机器合同（2026-10-01）

R04实现 `c365d5569cb94cc9bade2d17499a37040ae9736a` 与文档收尾
`4306eb9469e9ffa38ac1f9179bfa11607f3be211` 已核对，收尾后工作区clean；保持原分支。
开始A02独立实现，当前未提交。只读盘点Core/transport、service_error宏及手写service
errors、CLI/HTTP/C/Python边界与stream错误处理。发现已有部分flattened String错误
不能恢复原始类别/链，WebInspector startup把timeout/reset归为Disabled；不得通过
匹配自然语言猜测timeout/permission，也不能把尚未探测/metadata缺失当成不支持。

新增附加 `ErrorClass` 非穷尽enum与稳定snake_case code、`classify_error(&dyn Error)`
和 `CoreError::class()`；原CoreError/各service variants、字段、Display/source不变。
分类只查看typed variants/IO kind/真实tokio elapsed或cancelled join，最多32 sources，
source cycle有界，不调用Display/Debug；IO custom payload直接检查，避免source
forwarding跳过typed inner error。对已编译service按feature调用typed adapters；未知
device status保持ServiceFailure、flattened Other保持Unknown，EINTR/host file EOF
不猜为取消/设备断开。当前是分类草稿，不代表CLI/HTTP/Python/C已经获得机器字段。

首次all-feature check/test FAIL：MuxError与house_arrest原本没有crate-root reexport，
已改为现有内部/服务路径，不为测试扩大visibility；修正专项正在运行
（`/tmp/ios-a02-classification-fixed.log`）。新增原链/24层context、IO payload、codec
IO vs malformed data、cycle/no formatting、真实owned task取消/elapsed、固定机器codes
及AFC/WebInspector/media/clipboard/HID具名分支回归，未提前计为PASS。

修正后 `cargo test -p ios-core --all-features --locked error::classification`
**10 PASS、0 FAIL/ignored**（`/tmp/ios-a02-classification-fixed.log`）；最小feature同
filter **7 PASS**（`/tmp/ios-a02-classification-minimal.log`），原有minimal warnings
保留，不当成strict minimal Clippy。`cargo clippy -p ios-core --all-targets --all-features
--locked -- -D warnings` PASS（`/tmp/ios-a02-classification-lint.log`）；上述handles均
terminal exit0。新增外部consumer同时检查原CoreError exhaustive match、新classifier/
稳定JSON code与未来category fallback，最小consumer正在运行。初步
[分类合同](error-contract.md)明确stable codes、32-source bound、真实剩余信息丢失和
尚未接入的各入口，不宣称A02/第三类完成。当前未跑A02完整主线、未提交。

再新增附加 `RsdFeatureEvidence` 非穷尽enum，successful目录的exact/resolved查询
区分Advertised/NotAdvertised/MetadataUnavailable/ServiceMissing，canonical优先于shim，
ServiceDescriptor旧supports_feature的empty permissive行为不变。MetadataUnavailable
如实表示现有Vec不能区分omitted/empty metadata；NotAdvertised不是daemon明确拒绝。
没有把ConnectedDevice::rsd None（含skipped/failed）命名为NotProbed，真实probe状态
仍待owner维护。原classifier consumer最小feature **7 PASS**
（`/tmp/ios-a02-public-minimal.log`）；新增evidence consumer后最小 **8 PASS**、
全feature **10 PASS**（`/tmp/ios-a02-evidence-public-minimal.log`、
`/tmp/ios-a02-public-all.log`）。新API、合同与CHANGELOG同步，未增加绑定/CLI的字段。

`cargo test -p ios-core --all-features --locked rsd` **36 unit+2 selected integration
PASS**（AX一项、public consumer一项；其余filtered targets不计PASS），
`/tmp/ios-a02-rsd-regressions.log`。补充nested backup→DeviceLink→IO source回归后
分类专项 **11 PASS**（`/tmp/ios-a02-classification-final.log`）；相关handles均terminal
exit0，changed local links **67 PASS**。将首批Rust classifier/evidence作为可独立审查
slice验收，随后继续A02各入口/stream工作；当前完整gate待执行，不宣称主任务完成。

已启动首批slice完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`（`/tmp/ios-a02-classification-full.log`）；
当前未接受完整gate或提交。该阶段只新增Rust分类与RSD evidence查询，无CLI/HTTP/
Python/C machine字段或旧输出修改；后续source保留涉及旧variant赋值的变化必须单独
审查和记录迁移，不把附加分类误报成原信息已经恢复。

### 大类 3：A02 Rust 分类/evidence 首批交付（2026-10-01，主任务进行中）

实现 **`678fb9975fa3fe9747b8efb7e29996b31f33c745`**，11个明确相关文件；
提交后只留本计划。完整gate **PASS**（`/tmp/ios-a02-classification-full.log`），handle
已确认terminal exit0。该slice主机验收完成，A02/第三类未完成。

| 字段 | 实际记录 |
| --- | --- |
| 范围 | 附加typed ErrorClass/classify_error/CoreError::class，32-source bound；RSD successful snapshot的exact/resolved evidence query；两个真实外部consumer，原error variants/descriptor字段不变 |
| 相关验证 | 最终classification11 PASS、之前minimal7 PASS、public consumer最小8/全10 PASS、RSD36 unit+2 selected integration PASS；严格core Clippy及最终workspace Clippy PASS |
| 第11节主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`；1706 Rust/8 Python PASS、0 FAIL、2既有PBKDF2/browser ignored；66 features/4 groups、7+7 docs/3snippets、check/Clippy/MSRV、C ABI/19B raw、3.12 wheel/sdist重建安装、41tooling、118help、38raw、5host/22device NOT_RUN、8CLI/3backup PASS |
| 受测二进制 | ios0.1.13 SHA-256 `7fbab03cfb87b8b75ae0a96978b59ddc4d861696c2c3e0ca5cf78abaebc16f1c` |
| 审查/索引 | 三个旧public declarations/variants/fields逐字保留；不据此宣称ABI普遍等价；67 changed local links/whitespace PASS；[验收](validation/error-classes-2026-10-01.md)、[合同](error-contract.md)和Q01单项host evidence同步，无成熟度升级 |
| 真实剩余 | 原flattened strings/Disabled source丢失、未尝试/失败probe owner状态、CLI/HTTP/Python/C可编程机器错误、stderr callback/stream/partial语义均待实施；L01设备有序清理/取消/期限与平台/真机工作流待相应条件 |

无CLI/HTTP/Python/C状态或输出格式迁移；源码仅添加Rust API，不恢复既有已丢失
信息。没有真机、配对、DDI、TSS/擦除恢复、推送、发布或历史改写；browser与A01
3.9/3.13 wheel仍为历史独立证据。文档收尾后继续A02 source保留与机器错误边界，
总体目标active，第四类尚未开始。

### 大类 3 进行中：A02 WebInspector deadline source 保留（2026-10-01）

首批A02实现 `678fb9975fa3fe9747b8efb7e29996b31f33c745` 与文档收尾
`4ea5748ba725a52c7dd1495e5e1655cbb2386216` 已核对，收尾后clean。继续A02，当前
此slice未提交：CLI connection、application-list、launch、Safari-launch的四处
tokio timeout把丢弃Elapsed的map_err改为保留source的anyhow Context；已过期总预算
仍保留原context文本，但附加现有WebInspectorError::Timeout typed source。未改变
deadline数值、路由、真实协议、正常输出或exit，不宣称设备取消/cleanup ACK。

新增两个真实失败回归：expired预算的原context与typed Timeout；保留owned 8-byte
duplex peer不读，实际WebInspector application-list写入阻塞，观察真实Elapsed和
稳定Timeout分类，无设备dial。已fmt，`cargo test -p ios-cli --all-features --locked
cmd::webinspector::` 正在执行（`/tmp/ios-a02-web-timeout-source.log`），尚未接受结果。
该专项随后 **33 PASS、0 FAIL/ignored**，含原31条owner/access/实际Rust bridge回归与
新增2条typed source用例；handle已确认terminal exit0。此前初始“正在执行”是历史
启动状态，不解释为已经验收完整主线。
Core startup的Disabled合并和Ctrl+C字符串取消仍待各自source/迁移处理；后续机器
CLI/HTTP/C/Python字段、stderr/stream/partial与probe状态未完成。完整gate/diff/本轮
实现和文档提交仍待执行；A02/总体目标active。

### 大类 3：A02 CLI deadline source 验收（2026-10-01，主任务进行中）

实现 **`7fd97077ad67b474191a2ec64112a98c371fc980`**，三个明确CLI文件；提交后
只留本计划。专项33 PASS；完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，
`/tmp/ios-a02-web-source-full.log`，handle已确认terminal exit0。

| 字段 | 实际记录 |
| --- | --- |
| 分拆主线 | 1708 Rust/8 Python PASS、0 FAIL、2既有ignored；66 features/4 groups、7+7 docs/3 snippets、check/严格Clippy/MSRV、C ABI/19B raw、3.12 wheel/sdist重建安装PASS |
| 工具/进程 | 41 tooling、118 help、38 raw、5 host/22 device NOT_RUN、8实际CLI/3 backup合同PASS；owned cleanup无secret hits/设备dial |
| 受测二进制 | ios0.1.13 SHA-256 `ace50f42c398f3f221bc53cab0fc9c4b8122826f6ec84f369ba0cb6d2d85cfb5` |
| 审查 | 四处真实Tokio wrapper保留Elapsed，一个已过期budget保留typed Timeout；原context、duration、路由、正常输出/exit保留；两个新增真实失败测试，原31项回归保留，diff/whitespace PASS |
| 索引/剩余 | [验收](validation/error-classes-2026-10-01.md)、[合同](error-contract.md)、CHANGELOG/quality/Q01单项host evidence同步；Core Disabled合并、Ctrl+C、probe owner、各入口machine字段、stderr/stream/partial及L01有序设备清理仍待实施/验收 |

该slice已接受，无成熟度升级，无真机、配对、DDI/TSS/恢复擦除、推送、发布或
历史改写。文档收尾后继续A02 Core startup source与机器边界；第三类/总体目标active，
第四类尚未开始。上段“完整gate待执行”保留为启动历史，以本条真实结果为准。

### 大类 3 进行中：A02 WebInspector startup typed source（2026-10-01）

deadline source实现 `7fd97077ad67b474191a2ec64112a98c371fc980` 与文档收尾
`aa2ba17da66439a67eafb875cc5fc0dc7caa1047` 已核对，收尾后clean。继续A02，先在
未修改startup算法的状态登记4条legacy真实duplex失败：closed write、report后EOF、
idle均Disabled，blocked report write仍Timeout；专项 **4 PASS**
（`/tmp/ios-a02-startup-legacy.log`，terminal exit0）。这不是device setting证据。

当前未提交：新增附加 `WebInspectorClient::start_preserving_errors`，共用原单deadline/
report/event算法，identity映射保留typed IO/Timeout；旧start仍保留原阶段性Disabled
映射、原public enum/fields不变。CLI/WebDriver/CDP八处startup选择新入口；成功wire/
output、failure exit/status/JSON shape保留，自动脱敏diagnostic的category/correlation
会因原始source恢复而变化，已安排迁移说明。新增4类对应source失败、两组旧/新健康
与malformed状态、下游exhaustive consumer，以及实际protected CDP/WebDriver startup
closed-stream失败回归；待结果。两次初始新测试compile FAIL仅为plist::Dictionary
FromIterator中的泛型.into推断，已明确String keys/Value::String后再执行，未计PASS。
相关专项、完整第11节、diff审查/聚焦实现与文档收尾仍待完成。原Core API尚可产生
Disabled，并非已恢复所有upstream strings/取消/probe/stream信息；A02/目标active。

startup专项最终 **19 unit+7 selected integration PASS**
（`/tmp/ios-a02-startup-final-focused.log`，其中本次10个core startup/1个public用例，
其余原start相关回归）；更精确WebInspector owner **30 unit PASS**、协议集成
**12 PASS**（`/tmp/ios-a02-startup-core-web.log`、
`/tmp/ios-a02-startup-wire-regressions.log`）。真实外部consumer仅webinspector feature
**10 PASS**、all features **11 PASS**；minimal既有warnings保留。CLI全WebInspector
**34 PASS**（`/tmp/ios-a02-startup-cli-fixed.log`），新增实际CDP/WebDriver HTTPclosed
startup保持500/unknown error并可见安全IO kind，含原成功/准入/撤销/桥回归。上述
handles均terminal exit0。CLI测试首次compile FAIL为有Drop的owner禁止struct update
move，改为明确所有fixture字段，未修改production ownership。源码检查确认旧startup
算法body只把mapper换成参数，error declaration未改；这是审查辅助，不是普遍ABI
或真机证明。完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` 已启动
（`/tmp/ios-a02-startup-full.log`）；尚未接受完整gate/实现提交。

### 大类 3：A02 startup source 验收（2026-10-01，主任务进行中）

实现 **`c01244f221189f0a77b1c2debb792f45ec7b151a`**，十个明确相关code/contract
文件；提交后只留本计划。完整第11节 **PASS**，
`/tmp/ios-a02-startup-full.log`、terminal exit0，专项结果见上条和
[验收索引](validation/error-classes-2026-10-01.md)。质量和Q01单项host evidence同步，
不升级WebInspector/CDP/WebDriver成熟度。

| 字段 | 实际记录 |
| --- | --- |
| 主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`；1720 Rust/8 Python PASS、0 FAIL、2既有ignored；66 features/4 groups、7+7 docs/3 snippets、check/严格Clippy/MSRV、C ABI/19B raw、3.12 wheel/sdist PASS |
| 工具/进程 | 41 tooling、118 help、38 raw、5 host/22 device NOT_RUN、8 CLI/3 backup合同PASS；owned cleanup/no raw secret hits |
| 二进制 | ios0.1.13 SHA-256 `7f19106fe92c32b658cd64d986b2c3434815eff1830a5df3f703a06511412ae0` |
| 审查/迁移 | 原start算法仅参数化mapper，enum不变；八处CLI adapters选择附加API；safe IO/Timeout source category/correlation变化，成功output与原failure exit/status/body形状保留；83 changed local links/whitespace PASS，详见error/public contract、usage/CHANGELOG |
| 真实剩余 | 原start仍可Disabled；其他flattened strings/Ctrl+C typed取消/probe owner/CLI HTTP Python C machine字段/stderr stream partial仍待实施；L01设备cleanup ACK/期限、真机及平台workflow待相应条件 |

该slice主机已验收，不宣称A02/第三类完成；无真机、配对、DDI/TSS/恢复擦除、推送/
发布/历史改写。文档收尾核对后继续A02 CLI可编程错误输出；总体目标active，第四类
尚未开始。前述pending主线以本条实际terminal结果为准。

### 大类 3 进行中：A02 opt-in CLI machine failures（2026-10-01）

startup实现 `c01244f221189f0a77b1c2debb792f45ec7b151a` 与文档收尾
`4683aa59e7cd0291d138244bfd7d6b45dc571283` 已核对，收尾后clean。继续A02，当前
未提交：附加global `--error-format=human|json`，默认human保留原final stderr/exit；
JSON v1只用于final failure stderr，含typed class、canonical/fixed phase、安全diagnostic、
exit_code、typed force confirmation_required、effects（operation unknown/input not_started）。
不修改stdout success格式、不承诺stream没有progress或cleanup ACK、不猜flattened
strings类别。Help/version成功仍走原Clap输出，输出失败保留exit1。

首次格式recovery专项 **1 PASS**（`/tmp/ios-a02-cli-error-recovery.log`，terminal exit0），
覆盖parser error、missing command、invalid mode、literal --分隔和子命令后的global选择；
只用同一Clap parser的ignore_errors收回已识别format，不扫描argv值/自然语言；应将
format放在错误参数之前，未识别/invalid format安全回落human。新增原源保留/原human
bytes、typed force/opaque unknown、input脱敏测试，machine_ filter已terminal exit0；
精确output专项待结果。实际CLI fixture新增带mode成功list、真实malformed plist reply、
human/JSON非法端口；拥有唯一usbmux endpoint/原始capture，待新binary执行。原8个
输出合同保留。机器validator新增wrong exit/混入文本/字段类型/原始secret负例，待结果。
CLI machine slice专项、完整第11节、diff审查和聚焦实现/文档收尾尚待完成；HTTP/C/
Python机器映射、Ctrl+C/probe owner和stderr/stream/partial仍待各自推进；A02/目标active。

最终CLI output专项 **11 PASS**（含原8条与新3条；
`/tmp/ios-a02-cli-output-unit.log`），machine filter **3 PASS**；strict CLI Clippy
**PASS**（`/tmp/ios-a02-machine-clippy.log`）。新增help/version格式recovery回归首次
**FAIL**：Clap DisplayHelp提前返回没有matches，已只对recovery parser递归禁用
help/version/display，主parser成功help照旧；最终selection **1 PASS**
（`/tmp/ios-a02-machine-selection-fixed.log`，另含nested help）。工具negative **8 PASS**
（原3+新5，`/tmp/ios-a02-machine-validator-final.log`），确保exit0、wrong class/status、
stdout/诊断混入、bad字段类型/原始secret无法PASS。上述handles均terminal exit0。

最终明确新binary build PASS后，实际 `python3 scripts/compatibility.py` **14 PASS**
（`/tmp/ios-a02-machine-actual-final.log`）：原8不变，新malformed真实plist、input
human/JSON、mode下正常list以及closed help/version JSON failure；所有owned cleanup
完成、0 raw secret hits，argument trap 0设备dial，ListDevices服务不路由设备操作。
之前12 PASS是未加入closed JSON help的历史subset，不当最终14证据。完整第11节
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`
已启动（`/tmp/ios-a02-cli-machine-full.log`），尚未接受完整gate或提交。合同/usage/
CLI README/public contract/CHANGELOG同步，machine schema为新opt-in可扩展v1。

首轮完整machine gate **PASS**（`/tmp/ios-a02-cli-machine-full.log`，terminal exit0）：
1724 Rust/8 Python、0 FAIL、2既有ignored；46 tooling/14实际CLI合同，其余主线保留。
该结果对应最终审查前版本，**不是后续修正后的提交证据**。审查发现旧human force
hint `.chain().any` 和新machine guard lookup无界；已共用32-source helper，补充有512
safety valve的cycle计数回归（避免regression本身hang）与24/40层guard contexts，
诊断/fields在bound内一致，不把false当已确认。合同记录原可能无界scan→有界hint
及deep unknown迁移；设备confirmation判断未改。新增专项与修改后的完整gate待结果，
未提交；A02/目标active，HTTP/C/Python/probe/cancel/stream真实剩余不变。

### 大类 3：A02 CLI machine failure 验收（2026-10-01，主任务进行中）

实现 **`70108b76ce12f5e8d726cb920c017f91c496c3c6`**，九个明确code/tool/contract文件；
提交后只留本计划。bounded修正后的output **13 PASS**，修改后完整第11节 **PASS**
（`/tmp/ios-a02-cli-machine-bounded-full.log`，terminal exit0）。
[验收索引](validation/cli-machine-errors-2026-10-01.md)及quality/Q01单项host evidence
同步，无成熟度升级。

| 字段 | 实际记录 |
| --- | --- |
| 主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`；1726 Rust/8 Python PASS、0 FAIL、2既有ignored；66 features/4 groups、7+7 docs/3 snippets、check/严格Clippy/MSRV、C ABI/19B raw、3.12 wheel/sdist重建安装PASS |
| 工具/进程 | 46 tooling、118 help、38 raw、5 host/22 device NOT_RUN、14实际CLI/3 backup合同PASS；bounded owned cleanup/no原始secret hits，input trap 0设备dial；list只请求ListDevices |
| 二进制 | ios0.1.13 SHA-256 `09198cbdcac76a953bb4eb5517c9e532e5ab8ec4ecae447dd317239b77d44b33` |
| 审查/迁移 | 新opt-in extensible JSON v1，原stdout success/human final/exit1/2保留；主parser未使用ignore_errors，recovery才禁用早display；human/machine guard共用32-source，深于bound不观察hint/guard而非授权；90 changed local links/whitespace PASS |
| 真实剩余 | machine option只转换final failure，普通stderr progress/tracing/stream仍可能混入；HTTP/Python/C字段、Ctrl+C typed取消、RSD probe owner和flattened strings待推进；partial stdout/effects unknown不承诺retry/rollback；L01cleanup/平台/真机workflow待各自条件 |

该slice主机验收完成，A02/第三类未完成；无实际设备操作、配对、DDI/TSS/擦除恢复、
推送/发布/历史改写。文档收尾核对后继续A02取消类型/绑定入口；总体目标active，
第四类尚未开始。首轮1724结果保留为修正前历史，不替代本条1726最终证据。

### 大类 3 进行中：A02 caller-observed cancellation（2026-10-01）

CLI machine实现 `70108b76ce12f5e8d726cb920c017f91c496c3c6` 与文档收尾
`f2ef3653d8d5e7437386170c2f557e81d65e4ffa` 已核对，收尾后clean。继续A02，当前
未提交：附加private-fields `OperationCancelled` Rust marker/constructor/root reexport，
classifier识别明确本地取消类型，plain EINTR仍Io；原CoreError variants不变。
WebInspector launch原Ctrl+C成功后的字符串error改为marker+原context，失败signal仍
保留IO，elapsed source仍保留。私有select helper接收caller-owned pinned future，
production仍传真实tokio Ctrl+C，原unbiased选择/deadline/owner作用域和output代码保留。

新增typed marker/24 contexts/explicit IO payload与EINTR negative、最小下游consumer，
CLI owned pending future的ready cancellation、failed signal、真实20ms elapsed、成功
结果测试；其中signal fixture是Rust-owned ready future，不是真实OS Ctrl+C或device
cleanup ACK。首次Core专项compile FAIL为外部consumer对Option<dyn Error>错误使用
assert_eq，已改is_none；修正专项/CLI专项、完整第11节、diff/聚焦实现与文档收尾
待结果。绑定字段、probe owner、stream/partial和L01有序设备清理仍待推进；目标active。

修正后Core cancellation filter **5 unit+1 public integration PASS**（其他targets
filtered，`/tmp/ios-a02-cancellation-core-fixed.log`）；精确typed classification专项
**12 PASS**（`/tmp/ios-a02-cancellation-classifications.log`）；默认最小features外部
consumer **9 PASS**（`/tmp/ios-a02-cancellation-public-minimal.log`，既有warnings不作为
strict minimal认证）。CLI全WebInspector **38 PASS**，含原34+新4（
`/tmp/ios-a02-cancellation-cli.log`）。上述handles均terminal exit0。marker、分类、
WebInspector source/CLI机器class迁移已写入error/public contract与CHANGELOG；无source
文本匹配。完整第11节已启动（`/tmp/ios-a02-cancellation-full.log`），待结果/diff/提交。

该完整gate **FAIL**（terminal exit1）：Rust/Python、check/Clippy/MSRV、feature/docs/C ABI/
wheel/sdist通过，tooling `test_owned_rust_error_debug_does_not_regress_to_payload_derivation`
拒绝新增OperationCancelled的derive Debug。marker目前只有private unit字段，但项目
owned-error规则要求安全Debug；已改固定类型名的显式Debug，新增public consumer断言，
保持该检查原规则不变。该FAIL不作为实现提交证据；安全检查专项和修改后的主线待
运行，目标active。

修正Debug后的diagnostic safety工具专项 **7 PASS**，all-features外部consumer
**12 PASS**（`/tmp/ios-a02-cancellation-safe-debug.log`、
`/tmp/ios-a02-cancellation-public-final.log`，terminal exit0）。新增实际CLI SIGINT
fixture：owned wrapper启动实际ios，唯一owned mux peer收取完整ListDevices后不回复，
只向自己的child发送SIGINT；观察exit1、JSON cancelled/effects unknown、peer EOF，
不能推进到pair-record/device service。最终binary重新build后，实际CLI合同
**15 PASS**（原14+new signal，`/tmp/ios-a02-cancel-actual-cli.log`），handle terminal
exit0、无原始secret hits、owned cleanup完成。该实际OS signal证明限定在host probe
等待阶段，不是真机或Apple teardown ACK。兼容validator原8测试仍PASS。
修改后的完整第11节仍待运行/结果、diff和提交；A02/目标active。

### 大类 3：A02 observed cancellation 验收（2026-10-01，主任务进行中）

实现 **`ace87c1d9f2276a78eb7518c6a8d2cb3ff5b54e6`**，十一个明确相关code/tool/
contract文件，提交后只留本计划。修正后的完整第11节 **PASS**，
`/tmp/ios-a02-cancellation-final-full.log`，handle terminal exit0；前述safe Debug gate
FAIL保留为历史，不当通过证据。[验收索引](validation/cli-machine-errors-2026-10-01.md)、
quality/Q01单项host evidence同步，无成熟度升级。

| 字段 | 实际记录 |
| --- | --- |
| 主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`；1732 Rust/8 Python PASS、0 FAIL、2既有ignored；66 features/4 groups、7+7 docs/3 snippets、check/严格Clippy/MSRV、C ABI/19B raw、3.12 wheel/sdist重建安装PASS |
| 工具/进程 | 46 tooling、118 help、38 raw、5 host/22 device NOT_RUN、15实际CLI/3 backup合同PASS；实际SIGINT只阻塞于唯一owned ListDevices endpoint，exit1/Cancelled/unknown effects/EOF，无pair或device service；owned cleanup与原始secret检查PASS |
| 二进制 | ios0.1.13 SHA-256 `7be336add56789ec4753be95de47d8f9964b9b9d59350fb310fef1ad23c9a053` |
| 审查/迁移 | 附加private-fields marker/显式fixed Debug、原CoreError variants不变；select原caller-owned pin/signal/budget/unbiased/成功输出保留；JSON class Unknown→Cancelled对应确实观察到signal，不宣称cleanup ACK；76 local links/whitespace PASS |
| 真实剩余 | HTTP/C/Python机器映射、RSD probe owner、其他flattened strings和普通stderr/stream/partial仍待实施；真实设备取消/有序cleanup与跨平台workflow待L01/相应条件；JS shell/其他服务取消路径不自动算本slice验收 |

该slice主机验收完成，A02/第三类未完成。无实际设备、配对、DDI/TSS/擦除恢复、
推送/发布/历史改写；文档收尾核对后继续A02 Python/C binding mapping。总体目标active，
第四类尚未开始。

剩余：最终分类专项/最小feature与public consumer验证，审查所有类型/状态来源；
补充service missing/metadata missing/unprobed/explicit unsupported的查询合同，CLI
stderr/stream/partial-failure清理及opt-in机器error、HTTP/C/Python映射与真实失败fixture；
修正可执行的source flattening而不破坏已发布variants；完整第11节gate、diff与阶段
实现/文档提交。A02/第三类进行中，总体目标active；第四类尚未开始。

### 大类 3 进行中：A02 Python error metadata（2026-10-01）

本轮继续时branch `feat/device-service-updates-20260927`，HEAD文档收尾
`364cdff64f5ea9ab396449311f8d8eff77912111`；仅 `crates/ios-py/src/lib.rs` 有上一回合
开始的相关未提交改动，未发现无关tracked变更。当前新增Python内建异常上的附加
`ios_error_schema_version`/`ios_error_class`/`ios_error_phase`/`ios_error_effects`：
device list和tunnel启动保留RuntimeError及原安全args，typed class、effects unknown；
invalid mode保留ValueError/原args且在设备调用前InvalidInput/not_started；无userspace
proxy保留原RuntimeError，Precondition/not_started。Rust-only GIL参数不添加Python
公开参数；metadata赋值失败继续返回异常并挂安全原exception为cause。

新增四个host回归：原safe args/typed Rust source、输入/proxy无资源保留、明确取消与
opaque文字/EINTR区分、metadata assignment failure仍失败。首次专项compile FAIL
（`/tmp/ios-a02-python-errors-host.log`）为abi3下PyString没有to_str方法，已改同文件
既有to_string用法，未计PASS。修正后的split host tests、installed wheel/sdist和
3.9/3.13签名/实际host失败、完整第11节gate、diff和聚焦实现/文档收尾待完成。
此次不把Python argument extraction/callback/分配异常算已全面映射；runtime初始化
和其他错误边界仍需审查。HTTP/C、probe owner、stream/partial及真机/L01条件保留，
A02/第三类/总体目标active，尚未开始第四类。

修正后的split `python3 scripts/validate.py python-host --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **12 PASS**（原8+新增4，
`/tmp/ios-a02-python-errors-host-fixed.log`，terminal exit0）。新增installed smoke以
owned subprocess/deadline和唯一loopback端点验证invalid-mode零接入、实际malformed
ListDevices reply、原signature与字段/脱敏。首轮package **FAIL**（
`/tmp/ios-a02-python-errors-package.log`，terminal exit1）：fixture错误假定diagnostic
绕过原CoreError::Mux wrapper直接以protocol error开头，已只检查保持不变的operation
prefix及独立typed字段；不改变production映射、不靠消息推断class。修正package、
最低/较新Python、完整主线和提交仍待执行。

修正后的 `validate.py python-package` **PASS**，3.12 wheel/sdist分别重建、实际
安装后执行更新smoke（`/tmp/ios-a02-python-errors-package-fixed.log`，terminal exit0）。
按AGENTS.MD在 `crates/ios-py` 运行锁定解释器的 `uvx --from 'maturin>=1.5,<2'
maturin develop --locked` 及本地editable安装smoke，各PASS；同一新abi3 wheel
`/tmp/ios-a02-python-errors-wheel-8dbd6wxf` 用 `artifacts.py python --directory ...
--python /home/oslo/.local/bin/python3.9`/`python3.13` 分别新venv安装，3.9.25与
3.13.15均PASS（`/tmp/ios-a02-python-errors-py39.log`、
`/tmp/ios-a02-python-errors-py313.log`，terminal exit0），各覆盖公开signature、
invalid mode/零endpoint接入、真实owned ListDevices malformed失败的typed字段与
原始secret检查。Changed local links **77 PASS**，diff whitespace PASS。
本次改动没有运行S02独立浏览器gate；完整第11节已安排执行，尚未接受主线/提交。

### 大类 3：A02 Python metadata 验收（2026-10-01，主任务进行中）

实现 **`465b3208f9fd65ee4a106d9785872315c50648ff`**，六个明确相关code/tool/contract
文件；提交后只留本计划。完整第11节 **PASS**（
`/tmp/ios-a02-python-errors-full.log`，handle terminal exit0）；前述compile/package
fixture FAIL保留为历史，最终证据见[验收索引](validation/python-errors-2026-10-01.md)。
quality及Q01单项host evidence同步，无设备成熟度升级。

| 字段 | 实际记录 |
| --- | --- |
| 主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`；1732 Rust/12 Python PASS、0 FAIL、2既有ignored；66 features/4 groups、7+7 docs/3 snippets、check/严格Clippy/MSRV、C ABI/19B raw、3.12 wheel/sdist重建安装PASS |
| 包/进程 | 新abi3 wheel分别在3.9.25/3.13.15安装并执行同一smoke，editable develop/3.12 smoke PASS；invalid-mode owned trap零接入、唯一owned malformed ListDevices失败，fields/signature/原始secret检查PASS；46 tooling/118 help/38 raw/5 host/22 device NOT_RUN/15 CLI/3 backup合同PASS |
| 二进制 | ios0.1.13 SHA-256 `7be336add56789ec4753be95de47d8f9964b9b9d59350fb310fef1ad23c9a053`，未改CLI产品代码 |
| 审查/迁移 | 原RuntimeError/ValueError safe args与public signature保留；附加metadata v1可扩展字段/codes需fallback；操作effects unknown，mode/proxy拒绝not_started；native raw source不作Python cause；metadata赋值失败仍失败并保留安全cause；77 local links/whitespace PASS |
| 真实剩余 | Python argument extraction/interpreter/allocation/callback/runtime-initialization边界尚待审查；HTTP/C映射、RSD probe owner、其他flattened source与普通stderr/stream/partial仍待实施；真机/跨平台和L01有序设备cleanup待相应条件 |

该slice主机验收完成，A02/第三类未完成。无实际设备、配对、DDI/TSS/擦除恢复、
推送/发布/历史改写；文档收尾核对后继续A02 C binding mapping，HTTP等独立范围仍
保留。总体目标active，第四类尚未开始；前述pending full以本条terminal结果为准。

### 大类 3 进行中：A02 C caller-thread error category（2026-10-01）

Python实现 `465b3208f9fd65ee4a106d9785872315c50648ff` 与文档收尾
`62acdbe561e3ca56a3332a9ae100c8b9e5efe676` 已核对，收尾后clean。继续A02，当前
未提交：附加 `ios_last_error_class(void)`，返回borrowed static NUL code，不分配、
不释放，library loaded期间有效；每个caller OS thread独立，int-status calls开始
reset、成功none、失败typed category或诚实named-status fallback。void cleanup/
runtime init/u16 port/getter不改category；getter可在runtime init前使用。既有14个
symbol signatures/status0–8/layout/enum不变，新增第15个symbol。

七个Core/serialization failure边界在status转换前记录typed source，runtime的原String
flatten改为std::io::Error保留，numeric status仍6。NULL/UTF8→InvalidInput、state→
Precondition、caller buffer不足→ResourceLimit；IOS_ERR_ALLOC也可表示embedded NUL，
不猜为memory exhaustion而Unknown；panic7覆盖之前class为Unknown，不把已捕获panic
误报原cancel。新增4个source/实际metadata调用/线程隔离/static codes用例，原panic
用例扩展覆盖；专项、actual C link/owned malformed ListDevices、完整第11节、审查与
聚焦实现/文档收尾尚待验证。无interruptible C、device ACK或retry承诺；HTTP/Python
剩余/probe/stream/partial和真机平台条件仍保留，A02/第三类/总体目标active。

首轮C专项compile **FAIL**（`/tmp/ios-a02-c-errors-host.log`，terminal exit101）：
测试错误要求std::io::Error/CoreError实现RefUnwindSafe。测试中的只读owned error引用
用显式AssertUnwindSafe，线程fixture改为catch closure内部创建错误；production
`ffi_status` 的原UnwindSafe bound未减小，未计PASS。修正专项与后续gate仍待验证。

修正后的 `cargo test -p ios-ffi --all-features --locked` **19 PASS**（原15+新增4，
`/tmp/ios-a02-c-errors-host-fixed.log`，terminal exit0）；strict package Clippy
**PASS**（`/tmp/ios-a02-c-errors-clippy.log`，terminal exit0）。新C error fixture工具
negative **3 PASS**，拒绝empty/wrong exit/unexpected stderr/unsafe capture、timeout/
overflow/incomplete/cleanup失败和无peer证据（`/tmp/ios-a02-c-errors-validator.log`）。
实际 `validate.py ffi` **PASS**（`/tmp/ios-a02-c-errors-linked.log`，terminal exit0）：
strict C11 compile/link/例子、原19B smoke/raw，以及新25B owned malformed ListDevices
library call；status3+Protocol/output reset、后续NULL call替换class、void/u16不改
class、saved static pointer仍有效，0 raw hits/cleanup完成。源码核对原14 header
declarations、IosDevice/IosTunMode声明不变，新增getter；不是跨架构ABI普遍证明。
80 changed local links/whitespace PASS。完整第11节随后执行，尚未接受主线/提交。

### 大类 3：A02 C category 验收（2026-10-01，主任务进行中）

实现 **`3ae96914ae70cd70f4dcc62e1eafb0904af56b25`**，十个明确相关code/tool/contract
文件；提交后只留本计划。完整第11节 **PASS**（`/tmp/ios-a02-c-errors-full.log`，
handle terminal exit0）；首次compile FAIL保留为历史，最终证据见
[验收索引](validation/c-errors-2026-10-01.md)。quality及Q01单项host evidence同步，
不升级设备成熟度。

| 字段 | 实际记录 |
| --- | --- |
| 主线 | `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`；1736 Rust/12 Python PASS、0 FAIL、2既有ignored；66 features/4 groups、7+7 docs/3 snippets、check/严格Clippy/MSRV、C ABI原19B raw+新25B owned typed failure、3.12 wheel/sdist重建安装PASS |
| 专项/工具 | FFI19 PASS；C11 strict header/link/例子、原14 signatures+新getter、status/layout、actual owned ListDevices malformed错误PASS；3 negative工具用例在3.14/3.9/3.13各PASS，完整49 tooling/118 help/38 raw/5 host/22 device NOT_RUN/15 CLI/3 backup合同PASS；owned cleanup/原始secret检查PASS |
| 二进制 | ios0.1.13 SHA-256 `7be336add56789ec4753be95de47d8f9964b9b9d59350fb310fef1ad23c9a053`，未改CLI产品代码 |
| 审查/迁移 | 原C status0–8/symbol签名/device layout/mode保持；新增same-caller-thread borrowed static code，int成功none/每次替换，void/u16/getter不替换、不释放pointer；七个native/serialization边界与runtime source分类；ALLOC/NUL不猜resource limit、panicUnknown；80 local links/whitespace PASS |
| 真实剩余 | HTTP映射、其他Python interpreter/callback/runtime与C过滤/void/port边界、RSD probe owner、flattened source及普通stderr/stream/partial仍待实施；C取消仅typed marker分类测试，不承诺interruptible call；runtime IO failure用构造typed源测映射，未诱导实际资源耗尽；真机/平台/L01cleanup待相应条件 |

该slice主机验收完成，A02/第三类未完成。无实际设备、配对、DDI/TSS/擦除恢复、
推送/发布/历史改写；文档收尾核对后继续A02 HTTP adapter，保留其余真实pending。
总体目标active，第四类尚未开始；前述pending full以本条terminal结果为准。

### 大类 3 进行中：A02 protected HTTP error metadata（2026-10-01）

C实现 `3ae96914ae70cd70f4dcc62e1eafb0904af56b25` 与文档收尾
`f4c4f6709ef742d5f2ef097e2105e351a1e947bc` 已核对，收尾后clean。继续A02，当前
未提交：在reviewed/router fallback HTTP error上附加固定 `x-ios-error-schema-version`
（1）/class/phase/effects（unknown）headers，成功不附加，原status/body与安全diagnostic
extension保持；phase只来自enum，不回显URL/query/headers。WebInspector AppError与
manager native失败在转换前typed classification；两个manager真实Elapsed dial分支
保留原message-only body并给Timeout class。Auth/Host/Origin denial是PermissionDenied；
ticket capacity ResourceLimit、input InvalidInput、entropy failure Unknown。无source的
reviewed/framework fallback保持Unknown，不按HTTP数字/自然语言猜设备类别。

仅向已准入Origin的error response暴露固定机器headers，不改变origin/credential/
preflight准入；HEAD正文仍空。尚未完成专项、实际protected native/fallback/denial
HTTP/原始secret验证、完整第11节、diff与聚焦实现/文档收尾。此slice不覆盖WebSocket
frame error、TLS/HTTP parser在router前失败、远端client source恢复或所有legacy static
manager类别；其余Python/C/probe/stderr stream partial与真机/L01条件保留，A02/第三类/
总体目标active，第四类尚未开始。

HTTP首轮existing fallback专项 **1 PASS**（`/tmp/ios-a02-http-error-first.log`），
加入body/header/成功负例与forged request/response headers后的owner专项 **2 PASS**
（`/tmp/ios-a02-http-error-headers.log`，terminal exit0）。新native WebInspector/
manager typed IO、cancel/EINTR/opaque/axum/JSON与真实Elapsed、原body/status回归待结果；
实际protected debug routes、完整access/manager专项及独立Chromium gate已安排。
浏览器probe首次 `node --check` **FAIL**（局部const denied与既有函数重名），已改
bootstrapDenial，最终syntax PASS。独立browser gate将实际fetch 401的四个机器headers
纳入原检查，同时保留relay/replay/reload/expiry/不准入Origin路径；未把syntax当browser
PASS。完整第11节和实现/文档提交仍待执行；A02/总体目标active。

2026-10-02 继续核对：branch/HEAD仍为 `feat/device-service-updates-20260927` /
`f4c4f6709ef742d5f2ef097e2105e351a1e947bc`，14个相关文件未提交，无无关tracked
变更。上一轮有实际code/doc改动、专项/真实Chromium证据，归类为progress；未标记
目标complete。旧Clippy handle已missing、log有Finished，未重启该旧进程；后续最终
code变更需重新gate。已确认native两项PASS、access28 PASS、WebInspector39 PASS、
manager40 PASS/1既有browser ignored；独立Chromium1 PASS，reload/expiry各16checks
并实际读取401的四个headers，旧syntax FAIL不当通过证据；上述专项handles已terminal
exit0。最终ticket7 PASS，加入issuer known outcomes及unknown status negative；未诱导
真实entropy exhaustion。此前默认fallback的未来未知status修为Unknown，非泛化Input。

继续补齐已有owner可明确判断的manager分支：hop/upstream容量限制ResourceLimit、
空selector/零port InvalidInput、registry/tunnel目标lookup miss NotFound、local
unsupported transport及legacy GET tombstone Unsupported。通过各分支传明确enum，
不扫描status/message做分类、不把HTTP/local Unsupported算设备显式不支持。原body/
status保持；新增最终专项、真实router/Chromium、完整第11节和提交待执行。A02/第三类/
总体目标active，其他source/probe/stream与真机平台条件仍保留。

最终manager专项首轮compile FAIL：新测试未导入既有 `JsonRequest` trait；修复后
运行40 PASS/1 FAIL/1 browser ignored，容量fixture的明文DNS URL先被S03保护策略
拒绝，未到达容量分支。改用可登记的HTTPS合成地址（不发起dial），保留安全策略，
最终同一完整manager范围 **41 PASS、0 FAIL、1既有browser ignored**，terminal exit0，
`/tmp/ios-a02-http-error-manager-final-accepted.log`。两次失败分别保存在
`/tmp/ios-a02-http-error-manager-final.log` / `-final-fixed.log`，不作为通过证据。
最终strict Clippy/独立真实Chromium正在执行；主线验收与提交仍待完成。

最终 `cargo clippy -p ios-cli --all-targets --all-features --locked -- -D warnings`
PASS（`/tmp/ios-a02-http-error-clippy-final.log`，terminal exit0）；最终独立browser
gate **1 PASS**（`/tmp/ios-a02-http-error-browser-final.log`，terminal exit0），
Node 24.19.0 / Playwright 1.63.0 / Chromium 151.0.7922.34，reload/expiry各16checks，
实际获准browser fetch读取四个headers。源码/专题文档diff与83个changed local links
通过；第11节完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` 已启动，log
`/tmp/ios-a02-http-error-full.log`，尚未得到最终退出码。主线运行期间不改产品代码，
仅只读核对下一项RSD owner：skip_tunnel/缺mdns和实际握手失败均曾返回None，
成功cached目录与on-demand重试也须明确区分，不能凭None声称尚未探测或设备不支持。

### 大类 3：A02 protected HTTP metadata 主机验收与交付（2026-10-02）

实现提交：`62d1c4ff41d94b9ec849e6cef3972b8d120bfb20`（14个相关文件）。证据：
[HTTP错误headers与真实Chromium](validation/http-errors-2026-10-02.md)。在受保护
manager/CDP/WebDriver router错误上增加固定version/class/phase/effects headers；
原status/body、安全诊断、成功输出与准入保留。native转换前分类，owner明确的
auth/Host/Origin/issuer/manager分支传typed enum；source未知保持Unknown，effects
始终unknown。已准入Origin实际可读headers，不扩大凭据/CORS/Host准入。

专项实际结果：shared HTTP2 PASS、native Web/manager2 PASS、access28 PASS、
WebInspector39 PASS、finalticket7 PASS、finalmanager41 PASS/0 FAIL/1既有browser
ignored；strict package Clippy PASS。最终独立Chromium gate1 PASS，Node24.19.0 /
Playwright1.63.0 / Chromium151.0.7922.34，reload/expiry各16 checks，terminal exit0。
保留首轮trait编译FAIL、明文DNS容量fixture提前安全拒绝FAIL及更早JS重名syntax
FAIL；修复fixture后通过，不将失败日志算通过。无真实entropy耗尽；unsupported
transport guard为dormant源码审查，未编造runtime或设备证据。

第11节完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，log
`/tmp/ios-a02-http-error-full.log`，terminal exit0；**1742 Rust/12 Python、0 FAIL、
2既有ignored**。fmt/check/strict Clippy/MSRV、66 feature builds/4groups、7+7
doctests/3downstream snippets、C19B ABI raw+25B owned failure、3.12wheel/sdist、
49tooling、118help、38raw、5host/22real-device NOT_RUN、15CLI/3backup通过。
受测ios0.1.13 SHA-256：
`a9b56fce623641603f0cd814aa37a78d23eed5d9ed0aa1543b0ed5613c450c4a`。
实现diff/whitespace和83 changed local links已审查，收尾更新能力证据/质量指标。

本HTTP host slice验收；A02/第三类/总体目标仍active。pre-router TLS/parser、
post-upgrade frame、远端/其他source、Python/C剩余boundary、RSD probe owner和
stderr/stream/partial继续；L01/平台/真机条件不因此解除。下一项推进RSD owner的
真实未探测/失败/成功观察，不用None或ProductVersion推断支持。未推送/发布/改写
历史/操作reference checkout或实际设备；按明确范围提交本类文档收尾后继续。

文档收尾范围：本计划、HTTP验收页、质量指标与能力证据四个文件；69个本轮文档
local links、`python3 scripts/capabilities.py` 和 `git diff --check` PASS。新增证据
仅host-unit/protocol，不升级任何能力的设备成熟度。实现提交内容已核对，产品文件
无剩余变更；本轮收尾随后做聚焦本地提交，总体目标保持active。

### 大类 3 进行中：A02 initial RSD probe owner（2026-10-02）

HTTP实现 `62d1c4ff41d94b9ec849e6cef3972b8d120bfb20` / 文档收尾
`e25eaf0cedd0df70a569ae18944a30fa5ec73781` 已核对，收尾后clean。继续真实
probe owner的独立可执行部分，当前未提交：新增借用的 `initial_rsd_probe()`、
private-field observation及non-exhaustive state/skip reason，区分初始跳过/成功/
失败，保存到达owner的CoreError/class，Debug只显示固定字段；skip_tunnel与缺mdns
分别登记。legacy rsd/into_rsd仍optional初始目录；on-demand retries保持原cache、
次数/期限/结果，明确不更新此历史snapshot，不把后续成功误记为初始成功。
private proxy dial保留IO而非Tls string，深层H2/RSD lost sources仍pending。

首轮 `cargo test -p ios-core --all-features --locked device:: -- --nocapture`
59 PASS、0 FAIL/ignored（`/tmp/ios-a02-rsd-initial-host-first.log`，terminal exit0）；
发现all-feature非test构建的internal skipped variant无构造warning，已按实际cfg
收窄，不抑制lint。补充真实owned不listen端口拒绝、非法端点零连接、实际skip路径
与外部consumer编译检查，最终专项/strict Clippy/第11节/diff/提交仍待完成。
一次apply_patch因fmt后的exact context不匹配失败，未部分写入，核对后修正。
A02/第三类/总体目标active；后续probe最新状态、inner source、binding、stream/partial
与L01/平台/真机条件保留。未操作实际设备或reference checkout。

最终owner fixture加入实际connection construction与legacy getter的一致性、保留typed
timeout/source和消费device后本地channel释放；只造owned handle，不创建TUN或
转发task，不当设备cleanup证据。最终all-feature device **63 PASS**，default device
**18 PASS**，external public-contract **12 PASS**；strict Clippy最终PASS，均terminal
exit0。tunnel-only前一轮41 PASS，新增owner fixture后的最终同范围与第11节完整主线
已安排顺序运行（`/tmp/ios-a02-rsd-initial-tunnel-only-final.log` / `-full.log`）；
未有最后退出码，不提前计PASS。源码/新owner文件/专题文档diff与81 changed local
links已审查；主线运行期间不改产品代码。

最终tunnel-only **42 PASS、0 FAIL/ignored** 后已进入full主线，fmt/check/strict
Clippy、Rust/Python拆分与MSRV当前PASS，feature矩阵仍运行。只读审查明确后续
可执行缺口：on-demand probe重试仍丢弃原失败并最终返回legacy Unsupported；
RSD bootstrap/read的H2错误仍在原public路径转换为Tls string；Python全局runtime
初始化仍使用expect，普通IO失败会进入panic边界；C list遇不可编码记录仍跳过后
报success。这些保留独立实现/兼容迁移与真实owned fixture验收，不算本initial slice
完成，不缩减A02。待full真实结束后提交与文档收尾，再推进相关host工作。

### 大类 3：A02 initial RSD probe 主机验收与交付（2026-10-02）

实现提交：`b2c002629b17fed302b087462eb0a955fdd23f4e`（12个相关文件）。证据：
[初始RSD probe owner验收](validation/initial-rsd-probe-2026-10-02.md)。新增借用的
`ConnectedDevice::initial_rsd_probe()` 与private-field observation/non-exhaustive
state/skip enums；真实区分初始未尝试、成功、失败和两种跳过原因，保留到达owner的
CoreError/class，Debug只显示固定字段。原public字段/enum、rsd/into_rsd optional
初始目录、wire/retry/deadline/feature permissive行为保留；private proxy dial不再
把IOflatten为Tls string。后续on-demand重试不更新历史snapshot/cache，明确单列。

相关实际验收：finalall-feature device **63 PASS**、finaltunnel-only **42 PASS**、
default device **18 PASS**、external public contract **12 PASS**；strict package
Clippy最终PASS，均0 FAIL/ignored、terminal成功。owned真实ConnectionRefused保留IO，
非法端点零连接；实际skip owner、initial connection construction、原目录/typed
timeout与本地channel release、原owned proxy handshake/identity回归通过。没有TUN/
转发task或设备cleanup证据。首轮59 PASS的internalvariant warning与一次patch
context失败保留记录；不压lint、不编造设备/entropy/协议观察。

第11节完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，
`/tmp/ios-a02-rsd-initial-full.log`，terminal exit0；**1747 Rust/12 Python、0 FAIL、
2既有ignored**。fmt/check/strict Clippy/MSRV、66features/4groups、7+7doctests/
3downstream snippets、C19B ABI raw+25B ownedfailure、3.12wheel/sdist、49tooling、
118help、38raw、5host/22real-device NOT_RUN、15CLI/3backup通过。受测ios0.1.13
SHA-256：`29cb95dfa5729d22c7480db4500f08261fb69ce8af3f5ea9a468e9e48d98d90d`。
原始source/新owner文件/专题文档diff与81changed local links已核对；更新能力host
证据/质量指标，不升级设备成熟度。

本initial-probe host slice验收；A02/第三类/总体目标保持active。后续probe/retry
观察/原错误、inner H2/RSD source、Python runtime/interpreter/callback、C filtering/
void/port和stderr/stream/partial仍待实现，L01/平台/真机保持原条件。下一独立host
项处理Python Runtime普通IO失败进入expect/panic的问题；遵守GIL/既有signature、
safe typed metadata与真实打包验收，未授权任何实际设备操作。先完成本文四文件
文档收尾并核对提交/工作区，再继续；未推送/发布/改写历史或修改reference checkout。

本initial-probe文档收尾四文件的70 local links、`python3 scripts/capabilities.py`
与whitespace检查PASS；实现提交范围已核对，剩余只有本类文档。总体目标active，
本轮为真实实现/主线通过的progress，不能据此完成A02或31项总体目标。

### 大类 3 进行中：A02 Python runtime initialization（2026-10-02）

RSD实现 `b2c002629b17fed302b087462eb0a955fdd23f4e` / 文档收尾
`6f162c490d5e32de70c7b64ed9d6c4eeea89c96a` 已核对，收尾后clean。继续独立
host错误边界，当前未提交：Python LazyLock保存原 `io::Result<Runtime>`，普通IO
失败不进入expect/panic/poison路径、不调用device operation；runtime初始化及blocking
work仍在allow_threads内，invalid mode先拒绝。新固定phase `initialize runtime`，
native class、安全builtin RuntimeError和unknown effects，原signature/成功/native
operation args保持；异常初始化路径从panic变RuntimeError的迁移已写入专题合同。
真实thread/memory耗尽未诱导，其他runtime internal panic/interpreter/callback仍pending。

首轮split python-host **12 PASS**（`/tmp/ios-a02-python-runtime-host-first.log`，
terminal exit0）；新增cached failed initialization fixture：两次调用只初始化一次，
操作counter为0，原IO和safe Python type/args/metadata保留，正常runtime仍执行并保留
native failure；最终split **13 PASS、0 FAIL/ignored**（`-host-final.log`，terminal
exit0）。两次patch错误（invalid hunk/缺exact context）均未部分写入，核对后修正。
AGENTS要求的maturin develop、3.12 wheel/sdist、3.9/3.13安装、strict Clippy/第11节/
diff/提交仍待完成。A02/第三类/总体目标active，不操作实际设备或reference checkout。

editable `uvx --from 'maturin>=1.5,<2' maturin develop --locked` 与installed smoke
PASS（`-develop.log` / `-develop-smoke.log`）；3.12 `python-package` wheel/sdist
安装/接口PASS（`-package.log`），strict package Clippy PASS（`-clippy.log`），
均terminal exit0，82 changed local links/diff通过。首次3.9 artifact命令FAIL：引用
验证入口结束时已清理的临时run目录，未安装，3.13的&&后续未执行；log
`/tmp/ios-a02-python-runtime-py39.log`。新建本轮owned目录并构建一个abi3 wheel，
依次3.9/3.12/3.13安装验证；不重用已删除路径、不把该失败算通过。主线/实现与
文档提交仍待完成，其他条件保留，总体目标active。

新owned abi3 wheel目录 `/tmp/ios-a02-python-runtime-wheel-p1qwwo9j`，同一wheel
分别在3.9.25 / 3.12.13 / 3.13.15独立安装与实际binding smoke **PASS**，terminal
exit0；logs `-py39-accepted.log` / `-py312-accepted.log` / `-py313-accepted.log`，
build `-wheel-build.log`，未把前一临时目录缺失FAIL改写为成功。第11节full已启动，
log `/tmp/ios-a02-python-runtime-full.log`，尚待最终结果/提交；运行期间不改产品。

### 大类 3：A02 Python runtime IO 边界主机验收与交付（2026-10-02）

实现提交：`b064539f4e49830cad6b0c62a6efd9de21448529`（4个相关文件）。证据：
[Python runtime错误验收](validation/python-runtime-errors-2026-10-02.md)。普通
Runtime初始化IO失败由LazyLock保存原result、不进入expect/panic/poison路径、不
调用device operation；GIL释放和无效mode优先拒绝保持。返回safe RuntimeError的
typed class/固定initialize runtime phase/unknown effects，原signature/成功及native
operation args保持。异常初始化路径的exception-type变更有明确迁移/release记录。

split host **13 PASS**；strict package Clippy、AGENTS要求的maturin develop与
editable installed smoke、3.12wheel/sdist PASS；同一新abi3 wheel在3.9.25 / 3.12.13 /
3.13.15独立安装/实际binding smoke PASS，所有handles terminal exit0。controlled
failed initialization只构造owned typed IO：两次调用仅初始化一次、operation为0，
original source/safe type/args/metadata保留；真实current-thread runtime的owned
future/native error也通过。未诱导真实thread/memory耗尽，不把模拟当该真实失败
cleanup或设备startup证据。首次3.9引用已被工具清理的temp目录FAIL、&&后的3.13
未执行；新owned目录后通过，保留失败log；两次无写入patch错误同样记录。

第11节完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，
`/tmp/ios-a02-python-runtime-full.log`，terminal exit0；**1747 Rust/13 Python、
0 FAIL、2既有ignored**。fmt/check/strict Clippy/MSRV、66features/4groups、7+7
doctests/3downstream snippets、C19B ABI raw+25B ownedfailure、3.12wheel/sdist、
49tooling、118help、38raw、5host/22real-device NOT_RUN、15CLI/3backup通过。
受测ios0.1.13 SHA-256仍为
`29cb95dfa5729d22c7480db4500f08261fb69ce8af3f5ea9a468e9e48d98d90d`。
实现/专题文档diff和82changed local links已审查；只更新能力host证据/质量指标。

本普通runtime IO边界验收；A02/第三类/总体目标保持active。其他runtime内panic/
Python interpreter/allocation/argument/callback、C filtering/void/port、后续RSD
probe/inner source与stderr/stream/partial继续；L01/平台/真机条件保留。下一独立
host范围审查C list不可编码记录的显式结果/兼容迁移，先保留旧ABI与历史记录并
核对owned fixture可执行证据，不凭partial/empty success完成。未推送/发布/改写
历史/修改reference checkout或操作实际设备，四文件文档收尾后继续。

本runtime文档收尾四文件的71 local links、`python3 scripts/capabilities.py` /
whitespace PASS；实现提交内容已核对，剩余只有本类文档。总体目标active，本轮
三个已验收独立slice均为实际progress，不能据此完成第三类或31项总体目标。

### 续接会话：A02 checked C device-list projection（2026-10-02）

原会话 `01a0f318-99b9-7b71-bb7e-285621ed843b` 因额度停止；从本地会话记录
恢复完整31项目标和阶段提交授权，在新会话继续持久化目标。当前分支
`feat/device-service-updates-20260927`，恢复时HEAD
`d07e102434cdb7e27c1836a5ffdeafa790e73f2b`；五个未提交文件均属于本C slice，
无无关tracked变更，原实现保留并审查。已读取本地忽略AGENTS.MD、任务/验收/提交
规则及最新交付记录，不提交指南或操作参考checkout。

新增 `ios_list_devices_checked`，原layout/free/status保留；任何收到的UDID或
connection type含NUL时返回IOS_ERR_CORE/Protocol、NULL/0且无partial array。
旧entry保留skip兼容；prepared CStrings在检查期间由Rust拥有，失败释放此前行。
valid/empty保持success，这只检查收到列表的C投影，不证明upstream discovery完整。
头文件/示例/错误和兼容合同/Unreleased同步。owned binary-plist fixture经实际
linked library运行五次ListDevices，覆盖bad UDID/type、旧skip、新valid/empty。
工具扫描完整secret和NUL截断前缀，新增三项negative/positive validator tests。

原会话first baseline19 PASS、后续host21 PASS与linked fixture PASS已复核。
本次最终 `cargo test -p ios-ffi --all-features --locked` **21 PASS、0 FAIL/ignored**；
`validate.py ffi`三项19/25/26B raw captures及strict C11 header/link/example **PASS**；
strict package Clippy、fmt/whitespace、`validate.py tooling-tests` **52 PASS**，
均terminal exit0。日志 `/tmp/ios-a02-c-checked-list-{host,linked,clippy,tooling}-final.log`。
第11节full主线与提交/文档收尾仍待完成；主线期间不改产品代码。A02/第三类/总体
目标active，void/port、probe/inner sources与stderr/stream/partial继续；L01、跨平台
与真机条件保留。此fixture不接真实设备，不配对/DDI/TSS/刷机，不推送或发布。

### 大类 3：A02 checked C listing 主机验收与交付（2026-10-02）

实现提交 `6abb92f15e2dcd1b4a879ed15f5987bf71fdd1e5`（11个相关文件）；
[验收证据](validation/c-checked-list-2026-10-02.md)。恢复旧会话五文件工作并完成
兼容/错误/示例/工具收尾；新增checked entry以IOS_ERR_CORE/Protocol明确拒绝
含NUL的收到记录、NULL/0且无partial array。旧skip、status/layout/free保留。
仅验证C投影，不证明upstream discovery完整、实际设备可达或Apple cleanup。

最终FFI21 PASS、strict package Clippy/strict C11 header/link/example PASS；实际
linked五次binary-plist ListDevices覆盖bad UDID/type、legacy skip、valid/empty，
19/25/26B captures均zero raw hits、完整owned cleanup。三项新增validator使全工具
52 PASS；FFI专项工具Python3.9.25/3.13.15各6 PASS，仅工具不是新wheel安装。
15个已有header declarations完整保留，仅新增1个symbol；86 changed local links/
whitespace通过。CString先owned后转raw的失败释放经源码审查/实际错误路径验收，
没有heap profiler/sanitizer结论。所有专项与full handles均已terminal exit0。

第11节 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，
`/tmp/ios-a02-c-checked-list-full.log`；**1749 Rust/13 Python、0 FAIL、2既有ignored**。
fmt/check/strict Clippy/MSRV、66features/4groups、7+7doctests/3snippets、C ABI/
ownedfailure/checked、3.12wheel/sdist、52tools、118help、38raw、5host/22device
NOT_RUN、15CLI/3backup通过。ios0.1.13 SHA-256仍为
`29cb95dfa5729d22c7480db4500f08261fb69ce8af3f5ea9a468e9e48d98d90d`；独立Chromium
未因本C修改重跑，保留此前证据。四文件文档收尾更新host索引/指标，无成熟度升级。

本slice主机范围已验收；A02/第三类/总体目标active。下一项是RSD后续重试耗尽
丢失原失败/错误返回Unsupported的问题，补typed service lookup miss与兼容迁移、
owned fixture；inner H2/RSD source、C void/port与其余binding/stream/partial继续。
L01期限/取消、平台/真机条件不因此解除。没有推送/发布/改写历史/操作reference
checkout或实际设备，完成本类文档聚焦提交并核对工作区后继续。

### 大类 3 进行中：A02 RSD retry 与服务缺失 owner（2026-10-02）

C实现 `6abb92f15e2dcd1b4a879ed15f5987bf71fdd1e5` / 文档收尾 `d1d13df` 已核对，
收尾后clean。继续独立host source范围：RSD重试保留最后一次Completed失败，原
五次/500ms间隔/inner budgets与fresh transport保持，owner preflight仍立即返回。
successful cached/新discovered目录缺canonical/shim时用既有ServiceNotFound，
可判断ServiceMissing而不是笼统Unsupported；初始snapshot/缓存/metadata permissive
行为不更新。截图只为原特定服务增加typed fallback，legacy兼容分支保留。
proxy probe和XPC reconnector共用private typed dial/prelude adapter，原native IO/
非IO source保留。返回failure variant/安全correlation改变已记录迁移，无public
enum/字段/signature改动，inner H2/RSD flatten仍pending。

首轮all-feature device68 PASS、screenshot8 PASS、tunnel-only47 PASS、external
consumer12 PASS、strict Clippy PASS；随后补真实owned RSD discovery→directory miss/
EOF/零服务dial及native adapter source，最终all-feature device **70 PASS**。
新fixture使用真实TCP proxy prelude/H2/XPC，输入来自已有合成handshake形状，bounded
frames/总5s；无真实device/TUN。初始Failed/Timeout和cache仍保留，不把后续success
改写初始。一次apply_patch因fmt后的context不匹配失败，确认整批未写入后修正。
最终screenshot、tunnel-only/strict Clippy和第11节full尚待完成；A02/第三类/总体
目标active，C void/port、binding/stream/partial、L01/平台/真机条件继续保留。

最终专项all-feature device70 PASS、tunnel-only49 PASS、screenshot8 PASS，原external
consumer12 PASS；最终strict core/CLI Clippy PASS，均terminal exit0。85 changed local
links/whitespace与三个public enum/device声明文件逐字未变检查通过，实际diff已审查；
此source比较不是一般ABI或设备行为证明。第11节full安排在专项后执行，日志
`/tmp/ios-a02-rsd-retry-full.log`；尚无最终退出码，主线期间不改产品代码。

首轮full已terminal exit1：fmt/check/Clippy、1757Rust/13Python、MSRV/features/docs/
C与3.12wheel/sdist均先通过，但tooling中安全Debug静态guard检测到测试专用
AttemptSource派生Debug，52项工具有1 FAIL。该full不计PASS；去掉该derive并提供
固定安全Debug，不放宽guard。原失败日志保留；修复后专项/完整门禁重新执行。
源码只读确认下一项缺口：inner RSD/H2仍将typed错误转Tls string；syslog stream
Err诊断后break并返回Ok，应用部分mutation的global --json仍打印human状态。
这些不是通过证据，后续继续实际修复/合同及owned fixture，不据当前slice完成A02。

### 大类 3：A02 RSD retry/typed lookup 主机验收与交付（2026-10-02）

实现提交 **`233348554e44022f6bca522d18728d45b62bab05`**（8个相关文件）；
[验收证据](validation/rsd-retry-errors-2026-10-02.md)。原五次/500ms及inner budgets/
fresh transports保留，耗尽返回最后一次原CoreError/source，success/preflight早停；
successful目录miss返回既有ServiceNotFound/ClassServiceMissing，metadata/shim策略
和初始snapshot/cache不变。shared private dial/prelude adapter保留原sources，XPC
reconnector wiring源码审查，不声称完整断连重连workflow。截图按原特定服务typed
miss保留fallback；failurevariant/安全correlation迁移已写入合同，无public声明变化。

修复后all-feature device70 PASS、tooling52 PASS；screenshot8 PASS，external
consumer12 PASS（full重跑该consumer），strict core/CLI及fullworkspace Clippy PASS。
最终tunnel-only49在Debug修正前已通过，修正后的已提交代码安排accepted重跑，
结果另记。实际ownedproxy/H2/XPC discovery→empty目录miss/EOF/零service dial，
原Failed/Timeout snapshot/cache及本地watch release通过；无TUN/forwarding/设备
cleanup证据。原full在safeDebug guard有1 FAIL，保留失败日志，未弱化guard；改test
error为固定Debug后重新通过。一次patch context失败未写入的历史同样保留。

第11节修正后完整 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python` **PASS**，terminal exit0，
`/tmp/ios-a02-rsd-retry-full-fixed.log`；**1757 Rust/13 Python、0 FAIL、2既有ignored**。
fmt/check/Clippy/MSRV、66features/4groups、7+7doctests/3snippets、C19/25/26B
captures、3.12wheel/sdist、52tools、118help、38raw、5host/22device NOT_RUN、
15CLI/3backup通过。ios0.1.13 SHA-256：
`44d2553f82647b734705d76214a6031ea7246e2abd1eefd7aadcf7650ddd9db1`。
原始diff/whitespace/85local links和public声明逐字比较已审查，后者不是一般ABI证明。

本host slice验收，A02/第三类/总体目标active。下一项继续inner RSD/H2 native错误
在framer/bootstrap/read中转Tls string的source修复与兼容/fixture；随后stream
失败/partial、JSON mutation合同。C void/port、其他binding/相关L01/平台/真机条件
保留。四文件文档收尾更新host索引与指标，无成熟度升级；不推送/发布/改写历史/
修改reference checkout或操作实际设备。核对收尾后继续，不结束31项总体目标。

修正后已提交 `2333485` 的最终tunnel-only **49 PASS、0 FAIL/ignored**，terminal
exit0，`/tmp/ios-a02-rsd-retry-tunnel-only-accepted.log`；验收页同步最终结果。
四文件文档收尾73 local links、`scripts/capabilities.py` 118records及whitespace
PASS。实现范围/提交内容已核对，剩余仅本类文档；收尾后继续上述A02范围，目标active。

### 大类 3 进行中：A02 H2/RSD 原始错误来源（2026-10-02）

前一slice实现 `233348554e44022f6bca522d18728d45b62bab05` 与文档
`dfe2a67cf331f968142536b774e17e082d3205b2` 已核对，收尾后clean。继续修复inner
RSD/RemoteXPC的H2 setup/bootstrap/header/body/send/transfer错误转Tls字符串：
private mapper以现有XpcError::Io承载固定phase和原H2Error，native IO kind不变，
H2 Protocol为InvalidData且typed source保留。原enum/signature/frame/retry/预算及
pending limits不变；返回variant、Display前缀和safe correlation变化已登记迁移。

新增五条模块测试和一条公开root XpcClient consumer。合成AsyncRead/Write在
setup读/写、bootstrap/host handshake/send、header或已完整header后的body失败；
三种receive路径保留PermissionDenied/source，owned transport Drop精确一次。
没有打开设备、TUN或网络服务；pending bytes检查不认证设备cleanup或partial retry。
初次外部测试引用crate-private framer编译失败，已放回模块内并独立保留root API
consumer；首次四条来源断言未处理std::io::Error::get_ref而失败，修正遍历后
all-feature XPC **88 PASS**、public consumer **13 PASS**、tooling **52 PASS**。
失败日志保留；均未靠扩大API/弱化guard通过。tunnel-only、strict Clippy及第11节
full gate尚待完成；A02/第三类/总体目标active，其他binding/stream/partial继续。

### 大类 3：A02 H2/RSD source 主机验收与交付（2026-10-02）

实现 `96f23e91dbbdd79656586217bff766eff313801b`，9个相关文件，完整gate后提交。
24个H2-to-XPC adapters保留typed来源与固定phase，native IO分类恢复，H2 Protocol
仍Protocol；enum/signature/frame/bootstrap顺序/retry/预算/pending limits不变。
source观察包含std::io::Error::get_ref；Display/返回variant/correlation迁移已记录。

| 检查 | 结果 |
| --- | --- |
| 最终all-feature与tunnel-only XPC | 各88 PASS、0 FAIL/ignored；`/tmp/ios-a02-xpc-h2-source-fixed.log`、`/tmp/ios-a02-xpc-h2-tunnel-only.log`，terminal0；reduced-feature既有dead-code warnings保留 |
| public consumer | 13 PASS，原exhaustive matches与root XpcClient closed duplex的outer/native BrokenPipe来源通过；`/tmp/ios-a02-xpc-h2-public-final.log`，terminal0 |
| strict Clippy/tooling | Core all-target/all-feature PASS，tooling52 PASS；对应`/tmp/ios-a02-xpc-h2-clippy.log`、`/tmp/ios-a02-xpc-h2-tooling.log`，terminal0 |
| 第11节完整主线 | `/tmp/ios-a02-xpc-h2-source-full.log`，首次full terminal0；1763 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 其他完整gate | fmt/check/workspace Clippy/MSRV，66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查 | XpcError enum声明与旧commit比较相同；87changed local links/whitespace PASS。不是一般ABI证明；无额外Chromium或installed3.9/3.13重跑 |

owned faults覆盖setup、bootstrap/host handshake/send、完整header后的body失败与
三种receive；Drop精确一次、pending accounting零，不能证明partial安全重试、
connection复用或Apple cleanup。cryptex adapter wiring为source review+既有host
回归，非每个phase注入或真实transfer/TSS。两次测试开发失败与修正保留在
[专题证据](validation/xpc-h2-errors-2026-10-02.md)，未扩大private API或弱化guard。
ios 0.1.13 SHA256 `8f13a3de4261deaeb3e95a84fffd9b48de3dd5075661cc1086cae1371a6f23c8`。

本slice主机验收，A02/第三类/总体目标active。下一项syslog读取失败被吞成成功退出的
修复及流式/partial合同；application mutations的JSON、binding void/port及其他lossy
adapters继续。L01/L03、跨平台/真机条件保留；无设备操作/推送/发布/历史改写/
reference checkout变更。文档收尾后继续，第四类尚未开始。

### 大类 3 进行中：A02 syslog stream/partial 失败（2026-10-02）

H2/RSD实现 `96f23e91dbbdd79656586217bff766eff313801b` 和文档 `38509e2` 已核对，
收尾后clean。继续syslog：private driver共用原decode/filter/count/capture逻辑，
native读取错误立即返回主CLI failure path，代替诊断后break/Ok的exit0；EOF保持
UnexpectedEof/Io，不凭其推断device disconnect。compact NDJSON/原human字节保留，
逐record write+flush均检查，stderr固定progress也改为checked写；此前stdout保留，
output失败允许partial末record，capture时长/匹配条数正常停止继续success。

新增6条owned主机用例通过真实core.into_stream驱动相同CLI helper：4种native
failures在2条JSON后失败、EOF不输出未terminated记录、filter/count成功且drop、
idle duplex的实际1秒capture expiry与peer EOF、partial output/flush原IO、raw与
parsed旧bytes。失败JSON由实际shared renderer验证phase/class/exit1/unknown effects
和无private source，但未启动带配对TLS的真实syslog进程/设备，不认证signal或Apple
cleanup。首次human fixture误把既有parsed双newline当成单newline，产品字节未改，
修正expected后 `/tmp/ios-a02-syslog-host-fixed.log` **16 PASS**，strict CLI Clippy
`/tmp/ios-a02-syslog-clippy.log` PASS，均terminal0；失败log保留。

第11节full安排 `/tmp/ios-a02-syslog-full.log`，结果/提交/文档收尾尚待完成，主线期间
不改产品代码。A02/第三类/总体目标active，其他streams/JSON mutations、binding
void/port、lossy sources与L01/L03/平台/真机条件保留。只读另见core syslog的terminated
分支没有累计1MiB检查，需owned边界fixture确认/修复，未列为当前PASS范围。

首个full在Rust阶段失败：CLI **644 PASS/1 FAIL/1 ignored**，page bridge fixture
WebSocket被reset；`/tmp/ios-a02-syslog-full.log` 保留FAIL，后续gate未执行。孤立
同例和原两例成对5次均PASS，未完全复现失败来源；source显示page/browser两条独立
fixture共享process-global的PID:42:page-1 lock，100ms期限可能争用（此因果为推断）。
将browser owned page改为2并验证对应listing/setup/target，原期限/响应/cleanup
assertions保留；未修改product CDP lock。首个fixture修正用Drop struct的update
语法导致E0509，改为显式构造后四条bridge PASS、全部CLI **645 PASS/0 FAIL/1
ignored**、strict CLI Clippy PASS，均terminal0，分别
`/tmp/ios-a02-syslog-bridge-isolated-pages-fixed.log`、`/tmp/ios-a02-syslog-cli-final.log`、
`/tmp/ios-a02-syslog-clippy-final.log`。最终full重跑安排
`/tmp/ios-a02-syslog-full-fixed.log`，尚未接受；A02/总体目标仍active。

### 大类 3：A02 syslog stream/partial 主机验收与交付（2026-10-02）

独立fixture修正 `1de103eb79d66dad6905fdcb1e737d76a8f00c2d`（1个test-only文件），
syslog实现 `7b6394ec63dd68c79d290a1f470618ecf34110c3`（5个相关文件），完整gate后
聚焦提交。原read-error→break/Ok修正为保留native source→主CLI failure/exit1；
既有NDJSON/human字段与bytes、filter/count和capture停止保留，checked
write/flush与stderr progress，stdout此前records或partial末record不假称rollback。

| 检查 | 最终结果 |
| --- | --- |
| syslog/bridge/CLI | 16 syslog PASS，4 bridge PASS，全部CLI645 PASS/0 FAIL/1既有ignored；对应`/tmp/ios-a02-syslog-host-fixed.log`、`/tmp/ios-a02-syslog-bridge-isolated-pages-fixed.log`、`/tmp/ios-a02-syslog-cli-final.log`，terminal0 |
| strict CLI Clippy | `/tmp/ios-a02-syslog-clippy-final.log` PASS，terminal0 |
| 第11节完整主线 | `/tmp/ios-a02-syslog-full-fixed.log`，terminal0；1769 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/兼容 | private driver/render、公开args/fields/wire keys不变；103 changed local links/whitespace PASS；read/EOF退出码、safe failure、flush和partial迁移已写error/public/usage/CHANGELOG。非一般ABI证明；无额外Chromium或installed3.9/3.13重跑 |

6条新owned回归经过真实decoder，native失败在2条JSON后返回；EOF suffix不emit；
count/filter成功、reader Drop一次；实际1秒idle duplex capture与peer EOF；partial
write或flush失败保留原IO/终止emission；raw/parsed旧bytes精确保留。error由实际
shared renderer验证class/phase/exit1/unknown effects，dispatch wiring已source review；
非实际paired-TLS syslog进程/物理relay/OS signal/启动stderr失败fixture或Apple cleanup。
read UnexpectedEof为Io，native TimedOut为Timeout，与chosen capture success不同。

首次fixture newline误判、首次full bridge FAIL、未完全复现的global-lock争用推断、
Drop struct构造编译失败及保留assertions的isolated-page修正详见
[专题证据](validation/syslog-stream-errors-2026-10-02.md)。失败日志不记为PASS，修正
后的完整gate已真实结束。ios 0.1.13 SHA256
`f62516c5f070eb050dfb6cc9a5e95a8347e3b777678d4ef525ddb9fce684a3ed`。

本slice主机验收；A02/第三类/总体目标active，第四类尚未开始。下一项先复现/修复
core syslog terminated branch的累计message limit边界，再推进其他streams/JSON
mutations、binding void/port与lossy sources；L01/L03/平台/真机条件保留。无设备
操作、推送/发布、历史改写或reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 syslog limit 与 apps machine results（2026-10-02）

上一轮为progress：H2/RSD与syslog实现/fixture修正、完整gate及五个本地提交完成，
当前核对HEAD `00e439b`、分支 `feat/device-service-updates-20260927`、工作区clean。
不重做已验收范围，本轮先owned复现terminated final chunk绕过累计1MiB检查：
`/tmp/ios-a02-syslog-limit-before.log` 为真实1 FAIL。现统一在copy前检查至NUL的
body长度，exact-limit/next message保留；all-feature与syslog-only各7 PASS，CLI
真实decoder→driver的prior NDJSON+oversize Protocol/drop回归在17条syslog中PASS。
原S04/Q01/A02边界补齐，不认证error/cancel后的reader复用或decoded JSON上限。

同时修复apps mutation默认JSON打印human状态：private版本1/tagged单结果、pkill
ordered NDJSON per成功call与最终complete summary，--no-json旧bytes保留。原filter/
排序/逐process factory未改，send或output失败保留此前progress并阻止后续call，
no-match不发送、summary count0；counts仅为成功signal请求，非process exit。
六条renderer/owned call回归与named typed appservice ServiceNotFound fallback一条
通过，apps最终43 PASS。10处DTX与两处Elapsed来源保留，signal prerequisite保留
原CoreError；相同native timeout/permission/refusal/broken-pipe不触发fallback。
完整CLI/device mutation、真实DTX fallback、callbacks和L01/L03各自仍待条件。

对应专项日志 `/tmp/ios-a02-syslog-limit-after.log`、
`/tmp/ios-a02-syslog-limit-minimal.log`、`/tmp/ios-a02-syslog-limit-cli.log`、
`/tmp/ios-a02-apps-json-final.log`，terminal0；core/CLI strict Clippy专项PASS。
无实际device mutation/TUN/推送/发布/参考checkout改动。一次docs apply_patch context
不匹配无写入，核对usage后修正；guard移除unreachable allow后final apps重跑通过。
第11节full将执行 `/tmp/ios-a02-stream-apps-full.log`，结果/审查/实现与文档提交尚待。
A02/第三类/总体目标active；其他JSON命令/stream callbacks、binding void/port与lossy
sources继续，第四类尚未开始。

### 大类 3：A02 apps results/syslog bounds 主机验收与交付（2026-10-02）

实现 `60d5cb080f03d56194d9e9ea4d1ce8fd81dc49cf`（8个相关文件），完整gate后聚焦
本地提交。apps默认JSON的human mutation状态改为version1/tagged结果，pkill逐成功
signal request记录progress与最终complete summary，human旧bytes保留；原filter/
order/per-process factory与wire calls保留。输出或请求失败阻止后续call、保留此前
records，无complete summary/rollback/retry承诺；count不证明process已exit。
named typed appservice/shim miss fallback与legacy compatibility保留，10处DTX、两处
Elapsed及signal prerequisite来源保留。core syslog累计1MiB检查在final NUL chunk
copy前执行，exact-limit/next message及CLI prior NDJSON保持。

| 检查 | 最终结果 |
| --- | --- |
| 专项回归 | core all-feature/syslog-only各7 PASS，CLI syslog17、apps43 PASS；`/tmp/ios-a02-syslog-limit-after.log`、`/tmp/ios-a02-syslog-limit-minimal.log`、`/tmp/ios-a02-syslog-limit-cli.log`、`/tmp/ios-a02-apps-json-final.log`，terminal0 |
| strict Clippy | core/CLI all-target/all-feature专项PASS；`/tmp/ios-a02-stream-apps-clippy.log`，terminal0；最后fixture简化后的full workspace Clippy PASS |
| 第11节完整主线 | 首次full `/tmp/ios-a02-stream-apps-full.log` terminal0；1778 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/兼容 | private render/call helper；公开error enum/signatures/wire keys保留，JSON schema/partial/source/limit迁移写入error/public/usage/CHANGELOG；非一般ABI或physical workflow证明；无额外Chromium或installed3.9/3.13重跑 |

9条新增回归：真实core limit前失败与后边界PASS、CLI decoder→driver保留prior
record/Protocol/drop、typed named fallback、6条owned renderer/serial call/partial
failure/zero-match/human bytes。shared error renderer验证phase/class/exit1/unknown
effects/native source/safe diagnostics；application dispatch/source adapters已review，
但非实际paired-TLS CLI/appservice/DTX、process termination/staging或Apple cleanup。
首次oversize terminated regression真实FAIL、docs无写入context失败及final guard
处理详见[专题证据](validation/apps-stream-output-2026-10-02.md)。ios 0.1.13 SHA256
`24c44a2288cd7c2fa9ccd3d82a8426be576d81ee089ceb81010478a90d02043b`。

文档收尾：catalog118 records PASS，114 changed local Markdown links/whitespace
PASS；准确实现hash、范围与真实剩余条件写入catalog/quality/roadmap/专题证据。

本slice主机验收；A02/第三类/总体目标active，第四类尚未开始。其他JSON commands/
stream callbacks、binding void/port与lossy sources继续；streaming-install stderr IO
边界、reader error/cancel reuse、L01/L03/平台/真机条件保留。无maturity升级、设备
操作、推送/发布、历史改写或reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 C checked port snapshot（2026-10-02）

上一slice实现 `60d5cb0`、文档 `9822da2` 已核对，收尾clean；继续原binding void/
port边界，不重做apps/syslog。增设两个int-status checked port getter，沿用opaque
handle/既有status0–8/TLS类别：非NULL output先置0，NULL为NULL/invalid_input，
cached0为STATE/precondition，非0成功清category。原uint16 getter/zero fallback/
不更新类别语义保留；不凭缓存0猜kernel mode/skip/failed discovery/Unsupported，
不凭非0认证proxy/RSD当前reachability。无runtime、network、allocation或probe变化。
Rust两条metadata/actual exported getter tests含0/non0/mixed/1/u16MAX、NULL
handle/output、旧getter不覆盖类别、saved static pointer与真实caller thread隔离；
strict C smoke增signature/NULL output clearing。cbindgen export清单补齐新增symbols
及先前category/checked listing的遗漏；实际public header手工维护，未声称生成器验收。
专项FFI23 PASS、strict Clippy及linked C/owned malformed/checked-list PASS，分别
`/tmp/ios-a02-c-port-host.log`、`/tmp/ios-a02-c-port-clippy.log`、
`/tmp/ios-a02-c-port-abi.log`，terminal0。进度section初次误插到较早记录前，已移至
实际交付后，不改历史验收范围。第11节full安排 `/tmp/ios-a02-c-port-full.log`，
结果/审查/实现与文档收尾待完成。void init/cleanup、其他binding/source/JSON/
callback/L01/L03与device/platform仍待；A02/第三类/总体目标active。

### 大类 3：A02 C checked port 主机验收与交付（2026-10-02）

实现 `32a7e0653e36c336d153caed9772fc2db1a486cc`（8个相关文件），完整gate后聚焦
本地提交。新增两个int-status/caller-out checked cached port，明确NULL、cached0、
non0状态与TLS类别；legacy14接口、两个先前additive symbols、statuses/layout与
旧uint16不更新category语义保留。公开header、smoke和optional cbindgen exports一致；
仅name清单检查18 symbols，不声称运行generator或跨平台ABI验收。

| 检查 | 最终结果 |
| --- | --- |
| FFI专项 | 23 Rust PASS；`/tmp/ios-a02-c-port-host.log`，terminal0；strict all-target/all-feature Clippy `/tmp/ios-a02-c-port-clippy.log` PASS，terminal0 |
| C专项 | strict header/link/list example、NULL ABI/raw capture、owned malformed/checked list PASS；`/tmp/ios-a02-c-port-abi.log`，terminal0 |
| 第11节完整主线 | 首次full `/tmp/ios-a02-c-port-full.log` terminal0；1780 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/兼容 | cached0为STATE/precondition，不猜mode/metadata/probe/Unsupported；非0不认证reachability；输出先置0，same-thread类别/输出不可overlap/live handle不可concurrent close。无额外Chromium或installed3.9/3.13重跑 |

两条新增Rust通过真实exported functions覆盖0/non0/mixed/1/u16MAX、NULL参数、
category reset/old getter保留前类、saved pointer与fresh caller thread；ownedmetadata
无ConnectedDevice owner，非physical tunnel/proxy/RSD probe/support/cleanup证明。
strict linked C验证两新signature/NULL output，不伪称真实成功tunnel；所有原owned
listing/free测试保留。专题[验收证据](validation/c-checked-ports-2026-10-02.md)包含
dynamic/static/header/CLI SHA256；ios0.1.13仍为
`24c44a2288cd7c2fa9ccd3d82a8426be576d81ee089ceb81010478a90d02043b`。

文档收尾catalog118 records/104 changed local Markdown links/whitespace PASS，
准确实现hash/host范围/真实剩余条件写入catalog/quality/roadmap/专题证据。

本slice主机验收；A02/第三类/总体目标active，第四类尚未开始。void init/cleanup、
其他JSON/stream callback/binding/lossy sources、L01/L03/平台/真机条件保留，无maturity
升级、设备操作、推送/发布、历史改写或reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 C checked initialization/consuming cleanup（2026-10-02）

上一轮两slice/four commits为真实progress，当前HEAD `f239474`、分支
`feat/device-service-updates-20260927`、工作区clean已核对。继续void结果缺失边界，
新增runtime init与device/tunnel/string/list consuming release共5个int-status
entries；原void与uint16类别语义、symbols/status0–8/layout保留。checked init
分类cached普通IO，成功清类、caught unwind为PANIC/unknown，缓存失败不重试；
legacy init增加unwind guard而ordinary行为保留，未注入global initializer panic。
checked release接收caller owner storage，先NULL/0再release；NULL owner报错，NULL
owned resource success；list缺任一owner时均不修改。handle poison消耗后STATE/
precondition、caught release unwind消耗后PANIC/unknown；不能retry saved aliases。
cleanup也覆盖TLS类别，必须先保存原operation status/category。shared release
helpers供旧/新接口使用，不推断Apple cleanup/task join，L01/A03更强合同保留。

6条新增owned主机fixture：cached native runtime projection不执行operation、实际
global success/idempotent与旧initializer类别保留、NULL vs empty owner、真实allocated
lists/strings与拒绝owner不消耗、no-device handle正常/poison释放、shared helper的
counted destructor unwind/clear-before/drop-once。FFI29 PASS、strict Clippy和strict
linked C/actual owned ListDevices checked free PASS，分别
`/tmp/ios-a02-c-lifecycle-host.log`、`/tmp/ios-a02-c-lifecycle-clippy.log`、
`/tmp/ios-a02-c-lifecycle-abi.log`，terminal0。23 source/header/config names齐全，非
cbindgen运行或一般ABI证明；27专题本地links/whitespace/fmt PASS。一次docs patch
context不匹配无写入，核对后修正。第11节full安排
`/tmp/ios-a02-c-lifecycle-full.log`，结果/审查/提交与文档收尾待完成。
A02/第三类/总体目标active；其他JSON/stream callbacks、binding/source、L01/L03/
A03与真实平台/device/资源耗尽/全局poison/有序cleanup条件保留，第四类尚未开始。

### 大类 3：A02 C checked initialization/cleanup 主机验收与交付（2026-10-02）

实现 `5c0649a5b077db5ba78a34fdba8fb2b0e07cccfe`（10个相关文件），完整gate后
聚焦本地提交。5个additive int-status initializer/consuming cleanup，native cached
runtime来源分类、accepted owner清NULL/0后release、poison STATE/precondition与
caught unwind PANIC/unknown；saved alias不得retry，必须先保存operation类别再
checked cleanup。原14接口与4个前additions/status/layout保留，旧void cleanup/
uint16不更新category；legacy init增加unwind guard的迁移写入public/error/README/
CHANGELOG。examples检查init与consuming free；shared helpers无Apple/task-join证明。

| 检查 | 最终结果 |
| --- | --- |
| FFI专项 | 29 Rust PASS/0 FAIL/ignored；`/tmp/ios-a02-c-lifecycle-host.log` terminal0；strict all-target/all-feature Clippy `/tmp/ios-a02-c-lifecycle-clippy.log` PASS terminal0 |
| C专项 | strict23 signatures/header/link/example、NULL/no-op/runtime、raw capture、owned malformed/checked ListDevices/consuming free PASS；`/tmp/ios-a02-c-lifecycle-abi.log` terminal0 |
| 第11节完整主线 | 首次full `/tmp/ios-a02-c-lifecycle-full.log` terminal0；1786 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/范围 | 所有23 source/header/config names一致，不声称cbindgen运行或一般ABI验收。NULL required owner未消耗，accepted pointers先clear，saved static类别仍有效；physical/task join与actual global runtime fail/poison待条件。无额外Chromium或installed3.9/3.13重跑 |

6条新增owned fixtures：cached IO投影保持private原值且不执行operation，真实global
runtime repeated success/legacy category，NULL vs empty release，真实allocated list/
RSD JSON string及reject不消耗，正常/poison no-device handles，shared helper counted
destructor clear-before/drop-once/unwind与NULL no-op不再调用release。strict linked C
真实owned ListDevices/free包括缺owner仍保留outputs；一份valid legacy list继续旧free。
句柄无ConnectedDevice owner、panic仅shared helper destructor，不冒充真实设备Drop/
有序Apple cleanup。docs context首次fail无写入和精确SHA见
[专题证据](validation/c-checked-lifecycle-2026-10-02.md)；ios0.1.13 SHA256仍为
`24c44a2288cd7c2fa9ccd3d82a8426be576d81ee089ceb81010478a90d02043b`。

文档收尾catalog118 records/106 changed local Markdown links/whitespace PASS；
准确实现hash、可验证host范围与真实剩余条件写入catalog/quality/roadmap/专题证据。

本slice主机验收；A02/第三类/总体目标active，第四类尚未开始。下一项注册的streaming
install：core现有ProgressCallback为void，CLI startup/progress eprintln不保留output
IO失败；需要兼容入口/checked observer路径与native/partial fixtures，未列为当前PASS。
其他JSON/stream/callback/binding/source、L01/L03/A03与真实平台/device/全局poison/
资源耗尽/有序cleanup条件保留，无maturity升级、设备操作、推送/发布、历史改写或
reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 streaming-install checked progress（2026-10-02）

上一C lifecycle实现 `5c0649a`、文档 `5e9cdef` 已核对，收尾clean；当前分支
`feat/device-service-updates-20260927`。旧core ProgressCallback void与CLI eprintln
不能传播writer IO；新增兼容checked observer入口，原install_ipa委托同一wire flow，
callback failure立即保留ZipConduitError::Io/native source、不读取后续completion。
CLI startup/progress复用shared checked stderr writer，safe diagnostic bytes与既有
stdout install JSON保留。payload先于progress；错误不证明无device effects、rollback/
retry或borrowed stream可复用，L01/L03/device cleanup条件继续。

7新增fixtures：4条entire public install经tiny owned IPA/service，4种native callback
error后completion未读、legacy/checked/no-observer wire bytes与两种terminal保留、
prior progress后remote Error优先于Complete、truncated IO/invalid XML Protocol；2条
CLI shared writer partial write/flush/native source/safe actual envelope；1条external
consumer保留exhaustive enum/old-new callback signatures与local NotFound来源。
all-feature与apps-only core各4 PASS，CLI apps45、public14 PASS，strict core/CLI
Clippy PASS，日志 `/tmp/ios-a02-install-progress-core-final.log`、
`/tmp/ios-a02-install-progress-minimal-final.log`、`/tmp/ios-a02-install-progress-cli.log`、
`/tmp/ios-a02-install-progress-public.log`、`/tmp/ios-a02-install-progress-clippy.log`。
首个fixture编译E0283为Dictionary key.into类型歧义，改显式str；其后两configuration
各3PASS/1FAIL来自把single '?'解析器内层EOF误期望Protocol，产品native-first分类
未改，invalid XML integer与独立truncated输入现PASS。docs context错配无写入后修正。
第11节full将执行 `/tmp/ios-a02-install-progress-full.log`；结果/审查/提交/收尾待完成。
A02/第三类/总体目标active；其他JSON/callback/binding/ZIP string/lossy sources与
L01/L03/A03/平台/真机条件保留，第四类尚未开始，无设备操作/推送/发布/参考修改。

### 大类 3：A02 install checked progress 主机验收与交付（2026-10-02）

实现 `5810d16fa3eeaaf949b4226b80dfbc8c1871219d`（11个相关文件），完整gate后
聚焦本地提交。additive fallible observer保留ZipConduitError::Io/native sources，
callback Err立即return且不读下一reply/报success；legacyvoid callback/function与
exhaustive enum保留，委托同一transfer body。CLI startup/progress shared checked
stderr write/flush、原safe诊断bytes和stdout install successJSON保留；无成功结果
不证明无device effects。borrowedstream callback失败需discard，不自动close/join；
existingCLI ownerdrop不认证Apple cleanup/cancel完成，machine failure phaseapps/
exit1/unknowneffects，stderr不可写时最终报告仍best effort。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/CLI/public | all-feature/apps-only各4 PASS，CLI apps45、public14 PASS；`/tmp/ios-a02-install-progress-core-final.log`、`/tmp/ios-a02-install-progress-minimal-final.log`、`/tmp/ios-a02-install-progress-cli.log`、`/tmp/ios-a02-install-progress-public.log` terminal0 |
| strict Clippy | `/tmp/ios-a02-install-progress-clippy.log` core/CLI PASS，terminal0；final fixture后full workspace Clippy PASS |
| 第11节完整主线 | 首次full `/tmp/ios-a02-install-progress-full.log` terminal0；1793 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/范围 | 原body/wire keys/remoteError与completion顺序未改，legacy/checked/no-observer当前实现bytes相同；非priorrelease binary比对/一般ABI/physical install或cleanup证明。无额外Chromium或installed3.9/3.13重跑 |

7新增回归覆盖entirepublicinstall tinyownedIPA/service，4种native error在第二progress
之后completion仍未读，prior records/Debug/class/source保留；两种完成reply不call
observer、remoteError先于Complete、truncated nativeIO/invalidXMLProtocol原链；
ownedIPA目录清理显式验证。CLI partialwrite/flush与actual safeenvelope、external
old/new types/exhaustive enum/localNotFound保留。非pairedTLS CLI/physicaldevice/
borrow reuse/cancel或Applecleanup。E0283fixture歧义、两次3PASS/1FAIL parserEOF
误期望及无写入docscontext失败详见[专题证据](validation/install-progress-errors-2026-10-02.md)，
未弱化native-first分类；ios0.1.13 SHA256
`fb786aad30988485cee86ebb5882a1018495befcbfc704a658f784c7f640277a`。

文档收尾catalog118 records/121 changed local Markdown links/whitespace PASS；
locked plist1.7.1 is_binary的8-byte read_exact源码确认single '?'内层IO EOF来源。
准确实现hash、host边界及真实剩余条件写入catalog/quality/roadmap/专题证据。

本slice主机验收；A02/第三类/总体目标active，第四类尚未开始。下一已确认lossy
source为extract_zip_entries中的archive/entry ZipError与entry read IO→String，
需原source/class/variant迁移fixtures，非当前PASS。其他JSON/callback/binding/source、
L01/L03/A03/平台/真机/有序cleanup条件保留；无maturity升级、设备操作、推送/发布、
历史改写或reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 ZIP decoder 原始来源（2026-10-02）

上一install progress实现 `5810d16`、文档 `dea7c51` 已核对，收尾clean；当前
分支 `feat/device-service-updates-20260927`。extract_zip_entries三处String
adapter改为既有Io variant的private fixed-phase wrappers，保留原ZipError/IO
对象与nativekind。已知nonIO localpackage reasons仍InvalidInput，不推断device
unsupported/notfound；CRC原生InvalidData改为Protocol，未来ZIP reason Other/io
fallback。原public enum与old/new signatures、成功提取/transfer/wire未改，返回
variant/Display/context/correlation/class迁移写入public/error/CHANGELOG/rustdoc。

9新增fixtures：2条adapter保留7种IO及4种knownZIP reason的原对象/class/safeDebug；
5条realdecoder归档/条目头/密码metadata/CRC/valid-empty order，1条两public
install实际tinyownedIPA/service的4种decodefail-before-write/read/flush/observer，
1条CLI真实localdecode与safeenvelope apps/exit1/unknown。目录释放显式验证；
密码测试仅metadata标记，不认证加密payload。最初beforefix回归0PASS/1FAIL，
typedZIP source丢失已复现；expanded16PASS/2FAIL来自fixture误将localheader
预期为entryphase，locked2.4.2实际new时检查；改真实phase并补entrypassword
fault，产品native分类未改。日志 `/tmp/ios-a02-zip-source-before.log`、
`/tmp/ios-a02-zip-source-core-expanded.log` 保留。

专项all-feature/apps-only core各19 PASS，CLI apps46、public14 PASS，strict
core/CLI Clippy PASS（terminal0）：`/tmp/ios-a02-zip-source-core-final.log`、
`/tmp/ios-a02-zip-source-minimal-final.log`、`/tmp/ios-a02-zip-source-cli.log`、
`/tmp/ios-a02-zip-source-public.log`、`/tmp/ios-a02-zip-source-clippy.log`。
第11节full将执行 `/tmp/ios-a02-zip-source-full.log`，完整结果/提交/收尾待完成。
local解包发生在core payload之前，但CLI此前已service setup，effects仍unknown；
非physical安装/cleanup/streamreuse/newlimits证明，wholefile/decompression memory
未增加budget。其他A02 sources/JSON/callback/binding、L01/L03/L04/Q04/A03与平台/
真机条件继续，第三类/总体goal active，第四类尚未开始，无设备操作/推送/发布/
历史改写/reference checkout修改。文档收尾后继续。

### 大类 3：A02 ZIP decoder 来源主机验收与交付（2026-10-02）

实现 `b986ae1547bb16c2c90b4ee09b67817e6ab79d0a`（8个相关文件），完整gate后
聚焦本地提交。3处private adapter保留原returned ZipError/entryIO，经既有Io
variant/fixedphase；knownnonIO localinput语义保留，nativekind不再被String覆盖，
CRC InvalidData→Protocol迁移明确。原exhaustiveenum与函数/callback signatures、
success提取/metadata/wire不改，Debug/CLI安全、Rust cause原privatepayload保留。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/CLI/public | all-feature/apps-only各19 PASS，CLI apps46、public14 PASS；`/tmp/ios-a02-zip-source-core-final.log`、`/tmp/ios-a02-zip-source-minimal-final.log`、`/tmp/ios-a02-zip-source-cli.log`、`/tmp/ios-a02-zip-source-public.log` terminal0 |
| strict Clippy | `/tmp/ios-a02-zip-source-clippy.log` core/CLI PASS terminal0；full workspace strict PASS |
| 第11节完整主线 | 首次full `/tmp/ios-a02-zip-source-full.log` terminal0；1802 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/范围 | 9新增tests：真实decoder/2publicinstall在4类fault下zero write/read/flush/observer，native sources/classes、安全actualCLI envelope，valid-emptyorder；密码仅metadata、非加密payload/pairedCLI/physicalcleanup或newbudgets。无额外Chromium/installed3.9/3.13重跑 |

最初typed source回归0PASS/1FAIL与expanded16PASS/2FAIL原记录保留，后者为
localheader实际上在archive构造中检查，补entrypasswordmetadata并改真实phase，
产品分类未为fixture弱化。Locked zip2.4.2返回原InvalidArchive及CRC nativeIO
均保留；不声称重建库内部已隐藏原因，未知futurevariant fallback非构造验收。
原cause/class/context及returnedvariant migration、目录清理和精确日志见
[专题证据](validation/zip-decoder-sources-2026-10-02.md)；ios0.1.13 SHA256
`edfc4ad10aa102c37ca07b888a50e85690ecb7f521c85b0fb7bffd7a8ef508aa`。

文档收尾catalog118 records/106 local Markdown links（6个changed Markdown files）/
whitespace PASS；准确实现hash、host边界及真实剩余条件写入catalog/quality/roadmap/
专题证据。

本slice主机验收；A02/第三类/总体goal active，第四类尚未开始。下一已确认lossy
paths：CLI remote symbols connect、activation record application（包括内层
ConnectedDevice mobileactivation映射）、pasteboard XPC startup；须保留原sources/
class/context并验证migration，未列为当前PASS。其他source/JSON/callback/binding、
L01/L03/L04/Q04/A03、平台/真机/有序cleanup条件保留；无maturity升级、设备操作、
推送/发布、历史改写或reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 symbols/activation/pasteboard 来源（2026-10-02）

上一ZIP decoder实现 `b986ae1`、文档 `5b4bdcb` 已核对，收尾clean；当前分支
`feat/device-service-updates-20260927`。上一轮发现外层CLI strings及内层H2 /
modernactivation flattening；期间中断后核对clean/workers无残留，再落实修复。
remote symbols 16处H2/XPC adapters使用既有Io/private fixed phases保留原对象；
CLI connection context保持typed FetchSymbolsError。modern ConnectedDevice.activate
service mapping Other→Io保留MobileActivationError，CLI原CoreError直接经期限；
elapsed/observedCancellation/nativeSignal来源保留。pasteboard notification snapshot
connection Protocol→Xpc/Io保留原CoreError，并复用真实monitor callback。原enum/
signatures/Applekeys/requests/路由/回退条件/successoutput保留，返回variants与
Display/classes/correlation migration已写入public/error/CHANGELOG。

15新增tests：fetchsymbols7（2adapter、4entirepublicconnect/catalog/codec/file
fixtures、1closedduplexbeforefix），activation core3（2typedmapper、1真实modern
servicewritefail），CLI5（symbols1、activation3、pasteboard1）。symbols首个
beforefix0PASS/1FAIL曾Protocol替代nativeDisconnected；初轮core8PASS/2FAIL为
fixture10MiB单frame违反16KiB上限与非法控制在decode而非buffer阶段，修正输入和
phase不改产品limits/class。CLI初轮E0603因试图引用private xpc raw类型，改公开
IO cause检查而不暴露内部模块。日志 `/tmp/ios-a02-service-boundary-before.log`、
`/tmp/ios-a02-service-boundary-core.log`、`/tmp/ios-a02-service-boundary-activation-cli.log`
保留。所有source/future/peer均ownedhost，无实际设备或OS信号。

最终专项terminal0：symbols core全feature/minimal fetchsymbols各10 PASS，activation
core全feature/minimal mobileactivation各3 PASS，CLI activation11/symbols4/pasteboard12、
externalpublic14 PASS；strict core/CLI Clippy PASS。日志 `/tmp/ios-a02-service-boundary-`
前缀的core-final/fetch-minimal/activation-core-final/activation-minimal/
activation-cli-final/symbols-cli/pasteboard-cli/public/clippy.log。将执行第11节full
`/tmp/ios-a02-service-boundary-full.log`，完整结果/审查/提交/收尾待完成。其他A02
source/JSON/callback/binding、L01/L03/A03与平台/真机条件保留，第三类/总体goal
active，第四类尚未开始；无maturity升级、设备操作/推送/发布/历史改写/参考修改。

### 大类 3：A02 service boundary 来源主机验收与交付（2026-10-02）

实现 `817da67f9442e61f89e676a3dcfe93b3af71741a`（13个相关文件），完整gate后
聚焦本地提交。symbols16H2/XPC adapters与CLI连接保留原对象；modern record
application CoreOther→Io、CLI Core原链/Elapsed/observedCancellation/nativeSignal
来源保留；pasteboard snapshot connect Protocol→Xpc/Io保留CoreError并复用monitor。
原enum/signatures/requests/wirekeys/路由/回退条件/successoutput未改；returnedvariant/
Display/classes/correlation迁移及privatepayload显式Rust可见、安全Debug/JSON写入合同。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/CLI/public | symbols all-feature/minimal各10 PASS，activation core all-feature/minimal各3 PASS，CLI activation11/symbols4/pasteboard12、externalpublic14 PASS；`/tmp/ios-a02-service-boundary-`前缀core-final/fetch-minimal/activation-core-final/activation-minimal/activation-cli-final/symbols-cli/pasteboard-cli/public.log terminal0 |
| strict Clippy | core/CLI all-target/all-feature `/tmp/ios-a02-service-boundary-clippy.log` PASS terminal0；full workspace strict PASS |
| 第11节完整主线 | 首次full `/tmp/ios-a02-service-boundary-full.log` terminal0；1817 Rust/13 Python PASS、0 FAIL、2既有ignored |
| 完整其余gate | fmt/check/workspace Clippy/MSRV、66feature builds/4groups、7+7doctests/3snippets、C ABI/owned malformed/checked list、3.12wheel/sdist、52tools、118help、38raw diagnostics、5host/22device NOT_RUN、15CLI/3backup合同 PASS |
| 审查/范围 | 15新增tests覆盖真实publicsymbols与10MiB完整chunk后EOF/limitedsuccess、typed mappers/realmodernservicewrite、sharedCLI deadline/signal、realmonitorbaseline/peerEOF与safeenvelopes；非pairedConnectedDevice/ACK、actualOSsignal、deviceworkflow/cleanup或每个16stage逐一fault注入。无额外Chromium/installed3.9/3.13重跑 |

0PASS/1FAIL beforefix，初轮symbols8PASS/2FAIL非法10MiB frame/错误decodephase，CLI
E0603 privateXpc namespace错误引用记录保留，按实际16KiBframes/phase/publicIO
修正fixture，未弱化产品limits/class或暴露模块。localpeer/service/future drop证据
不认证devicecleanup或borrowedclient自动失效，failed符号下载可能留完整localchunks，
失败不返回successsize；最终机器effectsunknown。精确日志/host限制/未来条件详见
[专题证据](validation/service-boundary-sources-2026-10-02.md)，ios0.1.13 SHA256
`3f99a3449ee1bd86b00131c08808c186b49307ae4f9eec3b67d907b0a8301f3d`。

文档收尾catalog118 records/108 local Markdown links（6个changed Markdown files）/
whitespace PASS；准确实现hash、host边界和未来条件写入catalog/quality/roadmap/专题证据。

本slice主机验收；A02/第三类/总体goal active，第四类尚未开始。下一已确认
deactivation combinedfallback与activation HTTP errors仍flattened；需保留typed
原始来源/前后失败观察、合理class与兼容迁移。其他source/JSON/callback/binding、
L01/L03/A03、平台/真机/有序cleanup条件继续；无maturity升级、设备操作、推送/
发布、历史改写或reference checkout修改。文档收尾后继续。

### 大类 3 进行中：A02 deactivation/HTTP 来源（2026-10-02）

上一service boundary实现 `817da67`、文档 `d23a052` 已核对，当前分支
`feat/device-service-updates-20260927`；无其他运行任务或无关变更。ConnectedDevice
deactivate保持原一次fallback，modernservice映射保留原MobileActivationError；两次
失败经既有CoreIo/additive private-field DeactivationFallbackError保存两个CoreError，
包括fallback connection failure。borrowed getters与分类snapshots root/device公开；
source为最终failure，classifier采用其原class，opaqueUnknown不受第一failure覆盖。
fallbacksuccess仍Ok、primarysuccess不构造fallback，无新增retry。HTTP三处reqwest
flatten改nativeIo/privatefixedphases，四endpointrejections改typedInput/原URLparse
source，clientDebughashURL；原enum/signatures/HTTPSverification/optin/redirect/budgets/
wire/success保留，returnedvariant/Display/classes/correlation迁移写入合同。

14新增tests：coredeactivation4、HTTP8、externalpublic1、CLI1。真实modernservice
及legacyframe closedduplex、ownedTCPdial两cause，finaltyped/unknown、exactonce
fallback；HTTPSownedhandshakestall/refusal/realbuilder、本地ephemeralCA verifiedTLS
bodytruncation/bodytimeout/success/403及publicconstructor默认roots拒绝untrustedcert。
positiveTLS仅testprivateclient添加root，未关闭verification或新增生产CA接口。
CLI实际deadline helper保留publicHTTPnative error并渲染safeenvelope；非paired
ConnectedDevice/Lockdownsession/Appleendpoint/physicaldeactivation，不认证cleanup。

真实beforefix HTTP0PASS/2FAIL `/tmp/ios-a02-activation-http-before.log` terminal101：
timeout被HttpString归ServiceFailure、clientDebug泄漏query。新增deactivationfixture
初编译E0599误用不存在LockdownClient::new，改调用实际legacyframewriter而不新增
生产constructor；随后E0283 ambiguousDictionarykeys改明确str，原日志
`/tmp/ios-a02-deactivation-host.log`、`/tmp/ios-a02-deactivation-host-final.log` 保留。
首strictClippy items_after_test_module失败，移动测试至末尾而不allowlint：
`/tmp/ios-a02-deactivation-http-clippy.log`。产品语义未为fixture弱化。

专项最终：deactivation全feature/minimal各4、HTTPservice全feature30/minimal26、
externalpublic15、CLIactivation12 PASS；日志 `/tmp/ios-a02-deactivation-host-final2.log`、
`/tmp/ios-a02-deactivation-minimal.log`、`/tmp/ios-a02-activation-http-core-final.log`、
`/tmp/ios-a02-activation-http-minimal.log`、`/tmp/ios-a02-deactivation-http-public.log`、
`/tmp/ios-a02-deactivation-http-cli.log` terminal0。TLS四tests仅tunnelfeature下运行；
最小feature仍运行四HTTP tests。strictfinal及第11节full结果、审查/提交/收尾待完成。
第三类/总体goal active，第四类未开始；其他A02/source/JSON/bindings与L01/L03/
Q02/Q04/A03及平台/真机条件保留。HTTPbodybudget/status仍HttpServiceFailure，partial
readbufferzeroization有待L03，无devicecleanup/reuse认证或推送/发布/reference修改。

strictfinal已terminal0；首次full `/tmp/ios-a02-deactivation-http-full.log` terminal1，
fmt/check/workspaceClippy、1831Rust/13Python、MSRV、66features/4groups、docs/C ABI/
3.12wheel/sdist与52tools已PASS，随后catalog失败：旧source_inventory仅凭同名目录
推断mod.rs，误判仍有效的mobileactivation.rs。新增owned临时source-tree回归0PASS/
1FAIL `/tmp/ios-a02-service-inventory-before.log` terminal1；修复检查实际.rs入口再选
mod.rs，不调整原inventory记录、公开Rust入口或maturity。toolfixture增加至53，
专项及完整重跑 `/tmp/ios-a02-deactivation-http-full-final.log` 结果/提交/收尾待完成。

最终完整重跑terminal0：1831Rust/13Python、0FAIL/2既有ignored，fmt/check/workspace
strictClippy/MSRV、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/
checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、
15CLI/3backup均PASS。专项tools `/tmp/ios-a02-deactivation-http-tooling.log` 与实际
CLIinventory `/tmp/ios-a02-deactivation-http-inventory.log` terminal0。diff审查确认
无wirekeys/selector/requests/路由/fallbackschedule或HTTPSverification修改；原exhaustive
errorvariants保留，borrowedsource/snapshot与本地input/nativeHTTP分类迁移明确。
将按17个相关文件本地实现提交，精确hash/索引/收尾待下一记录。未额外Chromium/
installed3.9/3.13重跑，无paired/physical/remotecleanup/reuse证明，总体scope未缩减。

### 大类 3：A02 deactivation/HTTP 来源主机验收与交付（2026-10-02）

实现 `87e2b4789d1334ffe44e7b666d17c01eff8d949a`（17个相关文件），完整门禁通过后
聚焦本地提交。moderndeactivation保留原service来源；两次失败通过既有CoreIo/
publicprivate-field DeactivationFallbackError保存两个原CoreError、借用getter与分类
snapshot，最终fallbackcategory优先包括Unknown。nativeHTTP/URLparse来源保留，
endpointguards归Input、clientDebughashURL；原enum/signatures/一次fallback与HTTPS/
wire/success不改，returnedvariant/Display/classes/correlation迁移已记录。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/CLI/public | deactivation all-feature/minimal各4、HTTPservice all-feature30/minimal26、externalpublic15、CLIactivation12 PASS，相关日志见前一记录，terminal0 |
| strict Clippy / tools | core/CLI strict final PASS；53tooltests/actualCLI118inventory PASS，clippy-final/tooling/inventory.log terminal0；full workspace strict PASS |
| 第11节完整主线 | 最终full `/tmp/ios-a02-deactivation-http-full-final.log` terminal0；1831Rust/13Python、0FAIL、2既有ignored |
| 完整其余gate | fmt/check/MSRV、66featurebuilds/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup PASS |
| 审查/范围 | 新增14Rust+1tooltest；actualmodernservice/legacyframe与ownedTCPdial、dualcauses/finalunknown、verifiedlocalTLSbodyfaults/success/defaultrootsreject、CLIdeadline/safeenvelope和flatentryscanner。非pairedConnectedDevice/Lockdownsession/Appleendpoint/physicalcleanup/reuse；无额外Chromium/installed3.9/3.13重跑 |

HTTPbeforefix0PASS/2FAIL、fixtureE0599/E0283、strictClippytestmoduleordering，以及
首次fullcatalogsource-drift与ownedscanner0PASS/1FAIL均保留原日志；后者只修复实际
.rs入口识别，不改旧118capabilityrecords或maturity。精确证据与兼容/host边界见
[专题证据](validation/deactivation-http-sources-2026-10-02.md)，ios0.1.13 SHA256
`a0f1202357c39346e8e858dd39eb20e150ab6d5d37b46df52542ad38fd4f6547`。

文档收尾catalog118records/110localMarkdownlinks（6个changedMarkdownfiles）/
whitespace PASS；日志 `/tmp/ios-a02-deactivation-http-docs-links.log`，baseline
`d23a052` 含实现及收尾改动。准确implementationhash/host条件/后续待验收写入
catalog/quality/roadmap/专题证据，未升级maturity。

本slice主机验收；A02/第三类/总体goal active，第四类未开始。下一已确认lossy
paths：discovery与direct/remotepairing spawn_blocking JoinError、directTLS-PSK
connect、localcredentialhex/identity与remoteTLVbase64。须按原typed/native原因保留
source/class，明确input/protocol/unsupported及opaqueunknown，保持原requests/路由/
重试/信任政策；尚未作为PASS。其他A02HTTPbody-budget/status、JSON/callback/binding，
L01/L03/A03与平台/真机/有序cleanup条件继续；无设备操作、推送/发布、历史改写/
referencecheckout修改。文档收尾检查后继续。

### 大类 3 进行中：A02 connection/pairing 原生来源（2026-10-02）

上一deactivation/HTTP实现 `87e2b47`、文档 `a36853b` 已核对，分支
`feat/device-service-updates-20260927` clean后继续。盘点增加remote-discovery Join、
remoteTLS-PSK、AEAD与publickey/address nativeparser，覆盖20处原flattened/dropped
nativecause与3处Elapseddeadline adapters。既有CoreIo/privatefixedcontext保存原对象
和boundedclass snapshot，opaqueUnknown不因包装伪变IO；4Join、2verify、2PSK原类/
nativekind保留，3credentialinputs归Input，2Base64/2publickeylength/1IPv6/decrypt2
归Protocol，encrypt2仍nativeUnknown。Pskbackendunsupported保留原OpenSSLsource。
三dial预算保留Elapsed/TimedOut、固定context代替host/porttext，原预算/请求/路由/
重试/信任/成功/enum/signatures保持，returnedOther/Protocol→Io与类/Display/
correlation迁移写入合同。chacha20poly1305同版本开启std以保留nativeAEADError，
项目原已std/Tokio，服务feature/default/groups及cryptoparameters未变，Cargo.lock未改。

9新增Rusttests：privatecontext5、tunnel source4，另强化2既有endpointtests检验native
Elapsed/AddrParseError。涵盖9IOkinds/native/opaque、cyclebounded32/noformat、actual
asyncJoincancel+blockingJoinpanic、typedPairing/PSK/native/backendsource，真实PSK
ownedrefusal/closedduplex；actualmemorycredentialloaders2invalididentity/hex/mismatch、
2Base64functions、公钥0/31/33failure及32success，actualRemotePairinglistener overTCP
malformedB64/AEADauthenticationfailure/validencryptedport并join/drop。非真实credential
dirs/entirediscovery/paired/device/每个stage逐一fault注入；不声称runningblocking任务可
取消、remotecloseACK或borrowedclientreuse。InnerPairingErrorCrypto/cryptohelper仍有
Stringsource，未假称恢复；下一需继续其nativecause及其他A02/JSON/streams/binding。

beforefix初fixture多余JSONbrace编译101 `/tmp/ios-a02-pairing-source-before.log`；修正
fixture不改产品后真实0PASS/1FAIL UnknownvsProtocol `/tmp/ios-a02-pairing-source-before-final.log`
terminal101。core/CLIalltargetcheck `/tmp/ios-a02-pairing-sources-check.log` terminal0。
首strictClippy两err_expect失败 `/tmp/ios-a02-pairing-sources-clippy.log`，改有Debug的
ownedPSKresult使用expect_err，不suppresslint；final与追加publickeyparser后final2均0。

专项最终terminal0：context all-feature/tunnel-only各5、mdns-only3；tunnel all-feature17/
tunnel-only15；endpoint6、publicconsumer15、strictcore/CLI PASS。日志
`/tmp/ios-a02-device-context-core.log`、`/tmp/ios-a02-device-context-minimal.log`、
`/tmp/ios-a02-device-context-mdns.log`、`/tmp/ios-a02-pairing-sources-core-final.log`、
`/tmp/ios-a02-pairing-sources-minimal-final.log`、`/tmp/ios-a02-pairing-endpoint-core.log`、
`/tmp/ios-a02-pairing-sources-public.log`、`/tmp/ios-a02-pairing-sources-clippy-final2.log`。
第11节full `/tmp/ios-a02-pairing-sources-full.log` 正在运行，完整结果/审查/实现提交/
文档收尾待完成。A02/第三类/总体goal active，第四类未开始，全部31scope保持；无
真机/配对/Apple操作、推送/发布、历史改写或referencecheckout修改，平台/设备/cleanup
与L01/L03/A03各自继续待条件。

首次full terminal1：1840Rust/13Python、0FAIL/2既有ignored，fmt/check/workspace
strictClippy/MSRV、66features/4groups、7+7doctests/3snippets已PASS；FFIarchive构建
因磁盘NoSpace(os28)失败，不是C ABI验收PASS。root194G仅159M可用，target105G，
incremental68G；任务已终止并核对无Cargo/rustc残留。直接rm-f清理命令被执行策略
拒绝且未执行；改官方 `cargo clean -p ios-core` terminal0，移除64625个generatedfiles/
67.9GiB，恢复65GiB可用（source/reference/validationrecords未改）。日志
`/tmp/ios-a02-pairing-sources-disk-clean.log`、`/tmp/ios-a02-pairing-sources-disk-recovery.log`。
完整重跑 `/tmp/ios-a02-pairing-sources-full-final.log` 正在运行；source未为storagegate
weakening修改，结果/审查/提交/收尾继续。

最终完整重跑terminal0：1840Rust/13Python、0FAIL/2既有ignored，fmt/check/workspace
strictClippy/MSRV、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/
checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、
15CLI/3backup均PASS。source审查确认20native+3Elapsedadapters保留原因和原路由/
请求/信任/重试/预算，unknowncachebounded32不格式化，localInput/remoteProtocol角色
迁移明确。依赖stdfeature同版本/Cargo.lock不变且MSRV/minimal/downstream通过；
既有publicvariants/signatures未变。将按16个相关文件实现本地提交，再写准确hash/
索引与收尾；无额外Chromium/installed3.9/3.13重跑或真机cleanup/reuse认证。

### 大类 3：A02 connection/pairing 原生来源主机验收与交付（2026-10-02）

实现 `98aeccb5b4bab66d8431c1eeb9d91bbdacf02b62`（16个相关文件），完整门禁后本地
聚焦提交。20native+3Elapsedadapters通过既有Io/fixedcontext保留原对象/类别，
缓存bounded32分类包括opaqueUnknown，localcredentialInput与remoteProto分类明确；
PSKnativekind/backendsource保留，依赖AEAD stdfeature同版本、Cargo.lock不改。
原publicenums/signatures、wire/crypto/信任/路由/重试/预算/成功保留，变体/Display/
correlation迁移写入合同。未用诊断文本猜分类。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/public | context all-feature/tunnel各5、mdns3；tunnel all-feature17/minimal15、endpoint6、externalpublic15 PASS，相关日志见前一记录，terminal0 |
| strict Clippy | core/CLI final2 PASS terminal0；full workspace strict PASS |
| 第11节完整主线 | 最终full `/tmp/ios-a02-pairing-sources-full-final.log` terminal0；1840Rust/13Python、0FAIL、2既有ignored |
| 完整其余gate | fmt/check/MSRV、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup PASS |
| 审查/范围 | 9新增Rustfixtures+2既有endpoint strengthened；realasyncJoincancel/blockingJoinpanic/PSKownedfaults、memorycredential/B64/keylength、actualRemotePairinglistenerTCP3cases成功/故障+join/drop；非entirediscovery/pairing/device/directXPCstage逐一fault注入/runningblockingcancel或remoteACK/reuse |

fixtureJSONbrace编译失败、修正后的beforefix0PASS/1FAIL UnknownvsProtocol、首次
strictClippy err_expect2failure、首次fullFFI NoSpace(os28)均保留原日志。执行策略
拒绝直接rm-f未执行；官方cargo clean-pcore移除67.9GiB generatedoutputs恢复65GiB，
不动source/reference/validationrecords，完整重跑不弱化门禁或产品。最终证据见
[专题记录](validation/pairing-native-sources-2026-10-02.md)，ios0.1.13 SHA256
`833df7056404ca8dbf7cd6f16a1d15cf6759360cde49f11020f0eb1b8a4ee927`。

文档收尾catalog118records/112localMarkdownlinks（6个changedMarkdownfiles）/
whitespace PASS；日志 `/tmp/ios-a02-pairing-sources-docs-links.log`，baseline
`a36853b` 含本次实现和收尾；精确hash/host边界/剩余条件已写入catalog/quality/
roadmap/专题证据，不升级maturity。

本slice主机验收；A02/第三类/总体goal active，第四类未开始。下一确认innerloss：
HostIdentity privatekeyslice、HKDF expand、ChaChakeyinit/seal/open与OPACK encode
nativecauses仍String；需保留nativeobject、合理typedinput/protocol/unknown及兼容迁移。
另需核验当前PairingError::Protocol classification的tunnelfeaturegating在default/minimal
profile的一致性。其他A02/JSON/streams/callback/binding及L01/L03/A03与平台/真机/
有序cleanup条件继续。无maturity升级、设备操作、推送/发布、历史改写或reference
checkout修改。文档收尾检查后继续。

### 大类 3 进行中：A02 inner pairing 原生来源（2026-10-02）

当前源码核验纠正先前假设：lockdown::pairing模块/type本身cfg(tunnel)，分类器
同条件正确；default没有PairingError，未撤掉gate或假称复现default分类bug。
本模块Ed25519签名不返回错误，SRP proof验证返回bool，没有丢失的native
signature-validation error；实际另一丢失点为device-info OPACK encode。

修改前实际constructor/M6/device-info三个回归均失败：0PASS/3FAIL、terminal101，
日志 `/tmp/ios-a02-inner-pairing-before.log`。七个inneradapter保留nativecause，
seed/HKDF/keylength InvalidInput，receivedauthentication InvalidData/protocol，
encrypt opaqueUnknown，OPACK originalclass；既有PairingError::Io/fixedcontext/
boundedcachedclassification与safeDebug复用。Nestedcredential回归强化为PairingError
之下真正TryFromSliceError。HKDF同版本stdfeature使InvalidLength实现Error；原enum/
signatures/crypto32byte/salts/info/nonces/signing/wire/optionalM6/trust保留，明确
returnedvariant/Display/correlation迁移，不根据文本分类。

新增5innerfixture：实际seed0/31/33reject与32success、M6错误key/
tamper/truncate/nonce及正确altIRK、实际过长identity OPACK拒绝，真实HKDF/keyinit/
boundedBuffer AEAD错误经生产adapter、独立Python hmac固定向量和M5验签/字段。
后者nativeadapter不假称fixed32productionHKDF/keyinit或Vecencrypt失败注入。
首次专项配对16PASS；外部新增fixture误用crate-private lockdown路径，编译E0603
terminal101 `/tmp/ios-a02-inner-pairing-public.log`。已撤回该无效fixture、不扩API，
在内部fixture检查原exhaustive enum与Error/Send/Sync；既有external consumer重跑。
保留旧Protocol variant，对其已无producer的状态加单项有解释dead_code allowance，
不压制其他lint；合同准确区分内部变体迁移和外部CoreError来源/类别观察。
专项/full/审查/实现提交/文档收尾尚待；A02/第三类/总体goal active，第四类未开始。
其他A02与L01/L03/A03/platform/device条件仍保留，未操作设备或修改reference。

专项最终全部terminal0：pairing all-feature/minimal tunnel各16PASS；tunnel_connect
all-feature17/minimal15PASS；既有externalpublic full15/minimal10/default9PASS。
core/CLI all-target/all-feature strictClippy PASS，无全局allow或降低门禁。
日志 `/tmp/ios-a02-inner-pairing-{core,tunnel-core,public,minimal,tunnel-minimal,
public-minimal,public-default,clippy}-final.log`。第11节完整门禁开始，日志
`/tmp/ios-a02-inner-pairing-full.log`；source冻结，完整结果/审查/提交/收尾待完成。

完整门禁已通过fmt/check/workspace strictClippy、1845Rust/13Python（0FAIL、2既有
ignored）及MSRV1.80；66features/4groups与后续docs/C/Python/tooling等仍在运行。
实际diff审查七个转换保留原对象，不改32byte、salts/info、nonce、AEAD、signing、
wire/optionalM6/信任；公共错误类型和servicefeatures不扩展、Cargo.lock不改。
下一已确认lossy路径为默认配置也编译的lockdown plist/session PEM/rustls与version
转换，PairRecord plist Parse也丢source；须根据local/remote角色保留原cause/类别，
并保持TLS验证策略及success，不在当前slice或真机PASS中计入。

追踪actualcaller再确认：公开manual-pairing的pairing_transport仍有四处把
PairingError stringify成Crypto（含M6），其RSD/H2/XPC/bootstrap/AEAD/JSON也有
nativecause丢失。此slice仅innercause与nestedcredential保留，不假称完整公开
pairing流程已保留nativeclass；合同明确外层待补，优先续该调用链，再default
lockdown来源。Host-only、无新增trust/deviceoperation的边界保留。

完整门禁首次terminal0 PASS：1845Rust/13Python、0FAIL、2既有ignored；fmt/check/
workspace strictClippy/MSRV1.80、66feature编译/4groups、7+7doctests/3downstream
snippets、C ABI/ownedmalformed/checkedListDevices、3.12wheel/sdist安装、53tools、
118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup合同均PASS。日志
`/tmp/ios-a02-inner-pairing-full.log`。ios0.1.13 SHA256
`f7ba544e466b1c089bd86caeb652af0a02b3db99ae58f623a9b32dad1097dc3c`。
既有PBKDF2/browser ignored保留，无额外Chromium或installed3.9/3.13重跑。
八个相关文件diff/whitespace审查完成，准备本地聚焦实现提交，再准确hash文档收尾；
不升级maturity，不结束A02/第三类/总体goal。

### 大类 3：A02 inner pairing 原生来源主机验收与交付（2026-10-02）

实现 `9fa4ff78be9b30199188250ab63abb1869add4f6`（八个相关文件），首次完整门禁
通过后本地聚焦提交。七个adapter保留原seed/HKDF/keyinit/AEAD/OPACKcause，
分类localInput/remoteProtocol/opaqueUnknown与nativeOPACK，nestedcredential检验
完整TryFromSliceError；HKDF同版本stdfeature不改版本/lockfile/servicefeatures。
原types、params、signing、wire、成功与trust不变，内层变体/Display/correlation
迁移已记录。Protocol旧variant保留并仅对已无producer的该项作说明性allow。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/public | pairing full/minimal各16、tunnel full17/minimal15、existing external full15/minimal10/default9 PASS；日志见前条最终专项记录，terminal0 |
| strict Clippy | core/CLI及full workspace all-target/all-feature PASS，terminal0 |
| 第11节完整主线 | 首次full `/tmp/ios-a02-inner-pairing-full.log` terminal0，1845Rust/13Python、0FAIL、2既有ignored |
| 其余完整gate | fmt/check/MSRV1.80、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup PASS |
| 审查/范围 | 5新增innerfixtures与nestedcredential强化；actualseed/M6/OPACK failures、nativecryptoadapter、independentHKDF/M5signaturesuccess；非fixed32HKDF/keyinit/Vecencrypt failure injection、完整manualpairing/device/remoteACK/reuse |

beforefix0PASS/3FAIL与错误externalfixture E0603均保留原日志；后者撤回无效
privateimport，在内部保留enum/Error/Send/Sync检查，既有externalconsumer通过，
未扩API或弱化门禁。cfg(tunnel)与原signatureloss假设经源码校正。证据见
[专题记录](validation/inner-pairing-sources-2026-10-02.md)，ios0.1.13 SHA256
`f7ba544e466b1c089bd86caeb652af0a02b3db99ae58f623a9b32dad1097dc3c`。
catalog/quality/先前专题scope同步，未升级maturity。文档收尾catalog118records、
117localMarkdownlinks（7个changedMarkdownfiles，baseline a256d37）及whitespace
PASS；日志 `/tmp/ios-a02-inner-pairing-docs-catalog.log` 和
`/tmp/ios-a02-inner-pairing-docs-links.log`。full后无source变化，收尾聚焦提交待完成。

本slice主机验收；A02/第三类/总体goal active，第四类未开始，全部31scope保持。
下一优先pairing_transport四innerCrypto stringify与RSD/H2/XPC/bootstrap/AEAD/JSON
来源，然后defaultlockdown plist/PEM/rustls/version/PairRecord来源，保持wire与
TLS/trust政策，未列为当前PASS。其他A02/output/streams/binding与L01/L03/A03及
platform/device条件继续；未推送/发布、改写history、操作设备或修改reference。

### 大类 3 进行中：A02 manual-pairing transport 原生来源（2026-10-02）

已核对clean workspace/HEAD94373c1；前一turn为实现+文档提交和full gate的实际
进展。磁盘25GiB可用、incremental48GiB且无cargo/rustc/validate/maturin进程；
官方cargo clean-pcore terminal0移除28948 generatedfiles/34.1GiB，恢复58GiB。
日志 `/tmp/ios-a02-manual-pairing-clean.log`；source/reference/证据保留。

修改前实际write/read/unlock与shortheader四回归0PASS/4FAIL，terminal101；
`/tmp/ios-a02-manual-pairing-before.log`。write/read原Proto错误分类、AEAD原Unknown、
shortheader切片panic均复现。16个nativeadapter+1Elapsed保留RSD/bootstrap/XpcError、
H2Error、四PairingError、TryFromSliceError、AEAD、serdeJSON及dialElapsed原对象/
类别；nativeIO.kind保留，remoteauth/JSON/header Protocol，opaquecrypto/encrypt
Unknown，namedXPC service/closed精准。固定safecontext/boundedcachedsnapshot复用。
shortheader.get安全返回nativeSlice错误；actualH2仍读24byte。

原9variant/exhaustive rootPairingTransportError、rootfuture/signatures、credential
fields、requests/sequence/nonces/crypto/signing/bootstrap/stream/budgets/trust/
retry/remoteerror与unlocksuccess/optional保留，明确Xpc/Rsd/Crypto-to-Io与Display/
correlation迁移。checkedbodylen当前返回String非nativeError，原guard/variant/
diagnostic保留，未伪造source；此前已丢的legacyString仍不可还原。

新增8corefixtures+1externalrootconsumer；actualownedH2 closedwrite/header/body/
decode/bootstrap，unlockRPC AEAD/JSONfault+key/optionalsuccess，boundedpeer检查
request ID5/seq9与plaintext后shutdown/join/drop；actualRSD IPv6ownednonlistener
refusal/realElapsed，typedkind9项/namedXPC/opaqueAEAD与actualinnerM6/seed/SRP。
connect为actualH2call+productionadapter，不是entirepublicflow逐stagefault注入；
AEADencrypt用typednativefixture，未实际Vecfault；巨大encode-size与15snetworkdial
timeout未注入，无device/Trust/realcredential/Apple或remotecleanup/reuse证明。
externalrootAPI future仅typecheck不poll，保持正确private模块边界。

首all-feature pairing_transport18PASS，terminal0
`/tmp/ios-a02-manual-pairing-core.log`；minimal/public/strictClippy正在执行，full/
review/implementation与documentation closeout待完成。A02/第三类/总体goal active，
第四类未开始，31任务scope保持；接续defaultlockdown与其他source/streams/binding/
L01/L03/A03/platform/device条件，未升级maturity或推送/发布/改写history。

专项全部terminal0：all-feature/minimal tunnel,mdns pairing_transport各18PASS；
externalpublic full16/minimal11PASS；core/CLI all-target/all-feature strictClippy
PASS。日志 `/tmp/ios-a02-manual-pairing-{core,minimal,public,public-minimal,clippy}.log`。
源码核验dial常量15s，合同按实际值记录，不修改timeout。第11节full开始，日志
`/tmp/ios-a02-manual-pairing-full.log`；source冻结，完整结果/审查/提交/收尾待完成。

第11节full首次terminal0：1854Rust/13Python、0FAIL、2既有ignored；fmt/check/
workspace strictClippy/MSRV1.80、66features/4groups、7+7doctests/3downstream
snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist安装、53tools、118help、
38raw、5host/22deviceNOT_RUN、15CLI/3backup均PASS。ios0.1.13 SHA256
`fa4e97f1b3447e6584ad452dc259efd9f1a300c2eaf59bd443dff1f7b7c658c0`。
此前beforefix4FAIL保留，无额外Chromium或installed3.9/3.13重跑。八文件actualdiff/
whitespace审查完成；代码无依赖/feature/binding/CLI-schema变化，准备聚焦本地
实现提交和准确hash文档收尾；A02/第三类/总体goal继续，不升级maturity。

### 大类 3：A02 manual-pairing transport 主机验收与交付（2026-10-02）

实现 `d694cfafa392c3e465bef6bc2eb5e570bbbcaf75`（八个相关文件），首次完整门禁
通过后本地聚焦提交。16native+1Elapsedadapters保留原XPC/H2/Pairing/slice/AEAD/
JSON/Elapsed、nativekind和boundedcategory，包括四innernativechain；knownremote
Proto、opaqueUnknown与namedXPC classes明确。shortheader不再panic，原String
budgetguard保留。public9variant/rootfuture/fields/wire/crypto/bootstrap/trust/
15sdial/retry/success不变，返回variant/Display/correlation迁移已记录。

| 检查 | 最终结果 |
| --- | --- |
| 专项core/public | full/minimal tunnel,mdns各18 PASS；externalroot full16/minimal11 PASS，terminal0，日志见前条记录 |
| strict Clippy | core/CLI及full workspace all-target/all-feature PASS terminal0 |
| 第11节完整主线 | 首次full `/tmp/ios-a02-manual-pairing-full.log` terminal0，1854Rust/13Python、0FAIL、2既有ignored |
| 其余完整gate | fmt/check/MSRV1.80、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup PASS |
| 审查/范围 | 8新增core+1externalconsumer；actualclosedH2/send/header/body/decode/bootstrap/unlock/RSD、innerErroradapters/nativekinds/shortheader/realElapsed；RPC验证msg5/seq9/plaintext、AEAD/JSONfault与key/optionalsuccess+shutdown/join/drop；非entireSRP/device/allstage/encoder-size/Vecencrypt/network15s timeout fault/remoteACK/reuse |

beforefix0PASS/4FAIL（含原shortheaderpanic）原日志保留；安全scopedcargo clean移除
34.1GiB generatedcache、恢复58GiB并保留source/reference/证据，full未storage失败。
准确证据见[专题记录](validation/manual-pairing-sources-2026-10-02.md)，ios0.1.13
SHA256 `fa4e97f1b3447e6584ad452dc259efd9f1a300c2eaf59bd443dff1f7b7c658c0`。
catalog/quality/先前topic的历史边界同步，未升级maturity。文档收尾catalog118records、
123localMarkdownlinks（8个changedMarkdownfiles，baseline94373c1）与whitespace
PASS；日志 `/tmp/ios-a02-manual-pairing-docs-catalog.log` 与
`/tmp/ios-a02-manual-pairing-docs-links.log`。full后source未改，收尾聚焦提交待完成。

本slice主机验收；A02/第三类/总体goal active，第四类未开始，完整31scope保持。
下一defaultlockdown plist/PEM/rustls/version/PairRecord source；也需继续codec
integer/guard legacystrings、createdoutcomes、streams/JSON/callback/binding与
L01/L03/A03/platform/device条件。无推送/发布、history rewrite、设备或reference操作。

### 大类 3：A02 默认 lockdown 原生来源主机验收与交付（2026-10-02）

从clean HEAD `de8439406c2f7d83bfacaf30e381ab38b44a48ca` 接续；上一轮manual
transport的文档收尾已提交。实现 `d81fa3f5517dcfbd008dccc4bda963cff8e193c0`
（12个相关文件）已在完整门禁首次通过后本地聚焦提交。

九个adapter保留request/response plist、semver、两PEM IO、rustls client-auth、
DNS name、TLS IO和本地PairRecord plist；missingfile额外保留原生IO/OS code。
现有LockdownError::Io与PairRecordError::Read承载source，两个公开三variant
exhaustive enum保留。私有fixedcontext/cached32-source实现及三generic测试移至
always-compiled error模块，原source/Debug/snapshot行为保留，device内部re-export。
默认构建可保留opaque rustls Unknown；native TLS/文件kind保留，本地输入Input、
收到plist/version Protocol。受影响Protocol/NotFound/Parse到Io/Read、Display/
correlation迁移明确记录，未靠错误文本分类。

TLS provider/verifier/name/参数、requests/selectors、frame/file limits、Zeroizing
buffer/key、版本一/二段归一化、成功路径保持，无deps/lockfile/features/CLI/binding
变化或新增timeout/retry/trust。emptycert/missingkey/field/SSL/readahead等created
outcomes原Protocol保留，不伪造nativecause。

| 检查 | 最终结果 |
| --- | --- |
| 专项core | default lockdown31、full52 PASS；sharedcontext default/full各3、既有device2/manual18 PASS，terminal0 |
| 外部consumer/Clippy | root public_contract full17/default10 PASS；core/CLI与full workspace strict Clippy PASS terminal0 |
| 第11节完整主线 | 首次full `/tmp/ios-a02-default-lockdown-full.log` terminal0，1864Rust/13Python、0FAIL、2既有ignored |
| 其余完整gate | fmt/check/MSRV1.80、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup PASS |
| 审查/范围 | 九新增core+一external；actualserializer/frame/file/PEM/DER/name/version/TLS faults与file/framing/version/TLS appIOsuccess；bounded坏TLSpeer、两peer join/drop；非entirelockdownsession/device/trust/remoteACK/reuse/runningblockingcancel |

初fixture两轮lifetime/typeinfer编译错误修正后，未改production的九个runtime
回归2PASS/7FAIL，terminal101；原日志
`/tmp/ios-a02-default-lockdown-before{,-final,-runtime}.log` 保留。首次postfix窄测
30PASS/1FAIL因坏TLSfixture半个handshakeheader等待alert超时；修正为完整illegal
record后31PASS，原 `/tmp/ios-a02-default-lockdown-core.log` terminal101保留。
未弱化门禁/lint/timeout。默认窄测五个既有tunnel-MTU deadcode warnings保持，
all-feature strict PASS无新增allow。其余专项日志前缀
`/tmp/ios-a02-default-lockdown-`，准确文件见[专题记录](validation/default-lockdown-sources-2026-10-02.md)。

测试ios0.1.13 debugbinary SHA256
`754e4c9fb9618a340af415ccf0afb8d5393a3a1ce714eb64927a81c1e64d543b`。
无额外Chromium/installed3.9/3.13重跑；无storage失败/清理，可用约39GiB到28GiB。
source full后未变。catalog/quality/errorcontract与历史pairing专题followup同步，
未升级maturity。文档收尾118catalogrecords、103localMarkdownlinks（七个changed
Markdownfiles，baseline de84394）及whitespace PASS；日志
`/tmp/ios-a02-default-lockdown-docs-catalog.log` 和
`/tmp/ios-a02-default-lockdown-docs-links.log`。准备独立聚焦文档提交。

A02/第三类/总体goal active，第四类未开始，完整31任务/子任务scope保持。
接续supervised-pair OpenSSL/record-save原生来源，再codec整数转换/typedguards/
createdoutcomes、stream/JSON/callback/binding与L01/L03/A03/platform/device条件。
本片段未真实credentialstore/设备/Apple请求，也未推送/发布、改写history或编辑reference。

### 大类 3：A02 supervised pairing 原生来源主机验收与交付（2026-10-02）

接续clean HEAD `e452ea28652c109d620ede8f229198cfbcaf15ab`；上一轮default lockdown
实现与文档完成。实现 `b642afef32d8d575d45cbbdfd0a0ba7df8193b3a` 三个相关文件，
完整门禁首次通过后本地聚焦提交。本turn有实现/证据/提交进展，无阻塞审计触发。

原全部53native-to-string adapters（51 OpenSSL ErrorStack、1plist、1filesystemIO）
保留原对象/codes/entries，在现有LockdownError::Io和privatefixedcontext下分类：
48opaque backend Unknown，2local P12 Input，receivedpublickey Proto，save plist
原类别，write原IO.kind。另新增nativeNulError guard：DER后/parse2前拒绝含NUL密码，
替代已复现的dependencyCString unwrap panic；validpassword不新增CString copy。
nativeNulError持有密码bytes，明确source/Debug私有敏感，contextDebug仍safe。

Protocol-to-Io与Display/correlation/class迁移记录；公开三variant/error/root
functions/FullPairRecord fields与原features保持。Success/control-flow源码比对
剔除erroradapters/NULguard后完全一致；RSA2048、certificate fields/signatures/
validity/extensions、PKCS7BINARY、GetValue/Pair keys/order、privatekeystripping、
IDs/escrow/save/owner-only目录与replacement/tempcleanup原策略保持，无deps/lockfile/
CLI-schema/binding/newtimeout/retry/trust/lifecycle变化。createdmissingcert/key和
devicevalidation原Protocol保留，无伪造source。

| 检查 | 最终结果 |
| --- | --- |
| 专项core | minimal supervised-pair原13/final14 PASS；full lockdown61 PASS，terminal0 |
| 外部consumer/Clippy | final root full18/minimal11 PASS；core/CLI与full workspace strict Clippy PASS terminal0 |
| 第11节完整主线 | 首次full `/tmp/ios-a02-supervised-pairing-full.log` terminal0，1874Rust/13Python、0FAIL、2既有ignored |
| 其余完整gate | fmt/check/MSRV1.80、66features/4groups、7+7doctests/3snippets、C ABI/ownedmalformed/checkedlists、3.12wheel/sdist、53tools、118help、38raw、5host/22deviceNOT_RUN、15CLI/3backup PASS |
| 审查/范围 | 九新增core+一external；actualDER/password/NUL/remotePublicKey/root+signedcert/PKCS7/file faults；complete rootownedPair exchange，3request/64KiB frames，certsignature/knownsigner/binarychallenge/escrow、peer shutdown/join/hostdrop；save/Unixmodes；非physicalsupervision/Trust/enrollment/all51stage/allocator/fixedserializer/remoteACK/reuse |

beforefix八fixtures2PASS/6FAIL（含actualNulpanic），terminal101
`/tmp/ios-a02-supervised-pairing-before.log`；随后新增actualPKCS7 fault成为九fixtures。
首次external17PASS/1FAIL来自共享/tmp parentmode check先于预期NULpath校验，原
`/tmp/ios-a02-supervised-pairing-public.log` terminal101保留；核对/tmp为root1777，
无permission改动。fixture改为UUIDowned目录后final full18/minimal11 PASS，未改
production保存策略/门禁/lint。其余准确专项日志见
[专题记录](validation/supervised-pairing-sources-2026-10-02.md)。

测试ios0.1.13 debugbinary SHA256
`a8718b15ee20032bfe1e053bdf5255add2aa9c369d533a8c66bb9872bd328673`。
full source冻结且之后未变；无额外Chromium/installed3.9/3.13重跑，无storage失败/
cleanup（可用约28GiB至27GiB）。catalog/quality/errorcontract和default历史topic
followup同步，不升级maturity。文档收尾118catalogrecords、94localMarkdownlinks
（五个changedMarkdownfiles，baseline e452ea2）与whitespace PASS；日志
`/tmp/ios-a02-supervised-pairing-docs-catalog.log` 和
`/tmp/ios-a02-supervised-pairing-docs-links.log`。准备独立聚焦文档提交。

A02/第三类/总体goal active，第四类未开始，完整31任务/子任务scope保持。
已确认MCInstall使用同类P12 parse2（相同NUL panic boundary）并仍丢OpenSSL来源，
下一优先该入口，再base mux/archive/codec整数/typedguards/createdoutcomes、stream/
JSON/callback/binding、L01/L03/A03/platform/device条件；FullPairRecord原Vec secret
lifecycle保留待后续。未操作真实credentials/设备/Apple，未推送/发布/historyrewrite/
reference编辑。hostPKCS7 NOVERIFY/NOINTERN仅knownsyntheticsigner数学签名验证，
非chaintrust认证或生产策略更改。

### 大类 3：A02 MCInstall 原生来源与 P12 outcome 主机交付（2026-10-02）

接续 clean HEAD `1def8192b8d8952cfe56f4d35aa8020933bd13d1`。实现
`f1f9d37665d969011bd3b69970b03febe1ee0f45`，三个相关源码文件；完整门禁
通过后本地聚焦提交。本轮有实现、证据与提交进展，未触发阻塞审计。

六个 native-to-string adapters 移除：五个 OpenSSL ErrorStack 经现有 Io/
private fixed context 保留对象，remove-profile encoder 经现有 Plist 保留
原 plist::Error。P12 DER/password 为 Input，三个 opaque backend adapters
保持 Unknown。DER 后/parse2 前 native NulError guard 替代已复现的 dependency
CString unwrap panic；valid password 无额外 CString copy，native source 持有
密码 bytes，明确私有敏感访问边界。两个 feature-disabled 入口为 typed
CoreError::Unsupported admission；两个 missing cert/key predicates 为 Input
且不伪造 backend source。公开五 variants/root signatures/features 保持，
Crypto-to-Io/Protocol-to-Plist 及 class/Display/correlation 迁移记录。

Escalate success/control-flow 比对仅 error adapters/NUL guard/optional commas
变化；PKCS7 BINARY、DER/wire keys/order、challenge/既有 zeroization/ACK gating、
profile-list best-effort fallback 保持，无 deps/lockfile/CLI-schema/binding/
timeout/retry/trust/lifecycle 改动。metadata `u64 as i64` 留待 integer/typed-guard
审计，未猜测协议。原 Crypto(String) 手工构造仍 Unknown，PasscodeSet 为 Precondition。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core | mcinstall-only 25 / supervised 31 / all-feature 31 PASS，terminal 0 |
| 外部 consumer / Clippy | root all 19 / mcinstall 11 / supervised 12 PASS；core/CLI 及 full workspace strict Clippy PASS |
| 第 11 节完整主线 | 首次 full `/tmp/ios-a02-mcinstall-full.log` terminal 0，1883 Rust / 13 Python、0 FAIL、2 既有 ignored |
| 其余 full gate | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 覆盖边界 | 九 distinct internal fixtures 加一 external；actual feature/serializer/DER/password/NUL/missing-field/PKCS7/IO/plist/四 ACK faults；完整 public owned silent-install 四 requests/64KiB fixture frames、known signer/exact binary challenge、shutdown/join/drop；非 physical supervision/install/Trust/all backend stages/allocator/fixed encoder/remote cleanup/reuse |

修复前 final fixtures 两 feature modes 合计 5 PASS / 5 FAIL（九 distinct tests
的十次执行），含 actual NUL panic，`/tmp/ios-a02-mcinstall-before-final.log`
terminal 101。更早 before.log 中 empty-P12 fixture 的 native builder 失败已改为
真实 cert-only bundle；fixture construction 失败不冒充 production regression，
原日志保留。没有弱化 production policy、gate、lint 或 timeout。专项准确日志和
source comparison 见[专题证据](validation/mcinstall-sources-2026-10-02.md)。

测试 ios0.1.13 debug binary SHA256
`2319b64f23b52af9a96d3c820ed37ce189a4606b41c7b80163a64b0ef49e3034`。
源码 full 前冻结、之后未变；无 storage failure/cleanup，可用约 26GiB。
没有额外 Chromium/installed3.9/3.13 重跑。catalog/quality/error contract 和
历史 supervised topic follow-up 同步，maturity 不变；文档收尾检查另记录。

文档收尾：118 catalog records、五个 changed Markdown files 的 96 local links
（baseline `1def819`）及 whitespace PASS。日志
`/tmp/ios-a02-mcinstall-docs-catalog.log`、`/tmp/ios-a02-mcinstall-docs-links.log`。
源码不变，准备独立聚焦文档提交。

A02/第三类/总体 goal active，第四类未开始，全部 31 任务/子任务 scope 保持。
接续 base mux/archive native sources、codec integer/typed guards/created outcomes，
其他 stream/JSON/callback/binding、L01/L03/A03/platform/device 条件仍待。
未操作真实 credentials/设备/Apple，未 push/publish/history rewrite/reference 编辑。

### 大类 3：A02 usbmux 原生 codec/TCP/deadline 来源主机交付（2026-10-02）

接续 clean HEAD `adb95d9986d7f1e15400ea8235ed9d1064f855c7`，MCInstall 实现和
文档已完成。实现 `f8d890522798a444dde7dd600e954f4010f459c4`，五个相关源文件，
首次完整门禁后聚焦本地提交；有实际进展，未触发阻塞审计。

四个 native flattening adapters 保留原对象：request plist Input、received
plist XML/schema Proto、TCP connector 原 IO.kind/object/source OS code、deadline
原 Tokio Elapsed/Timeout，使用现有 MuxError::Io/std IO 和 private fixed context。
ConnectionRefused 仍 Io，UnexpectedEof 不猜作 Disconnected。三 error variants、
root MuxClient signatures/features 与 CoreError::Mux chain 保持。Protocol-to-Io、
send category Proto-to-Input、TCP fixed phase 替代 endpoint/duration prose 的
Display/correlation 迁移明确；raw native access 为私有数据，Debug 无 payload。

Production comparison 剔除四 adapters/private unused 参数/trailing whitespace
后一致：wire framing/limits/guards/tags/request fields/flush、10s TCP timeout、
Unix policy 和成功流程保持，无新 dependencies/CLI-schema/binding/retry/lifecycle。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core / external | mux minimal/full 15/15 PASS；root all/minimal 20/11 PASS，terminal 0 |
| strict Clippy / comparison | core/CLI 和 full workspace PASS；production comparison PASS |
| 第 11 节完整主线 | 首次 `/tmp/ios-a02-mux-full.log` terminal 0，1889 Rust/13 Python、0 FAIL、2 既有 ignored |
| 其余 full gate | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 五新增 internal、一 external type check、两既有 TCP fixtures 加 native assertions；actual serializer/XML/schema/closed write/truncated header/body/owned TCP refusal+OS code/Elapsed；public BUID owned TCP fault/success+shutdown/join/drop。非 physical daemon/device/production10s blackhole/cleanup/reuse/retry，root connect 未 poll |

Final before-fix 9 PASS/6 FAIL，`/tmp/ios-a02-mux-before-final.log` terminal 101。
更早 before.log 的 peer 错误期待 BundleID，改为原 serialized BundleId 后 final
重跑；没有改变 production wire。第一次比较 helper 的 test module 边界 trailing
whitespace 差异经 normalization 解决，无 production edit。失败记录不冒充 PASS。
准确专项日志和边界见[专题证据](validation/usbmux-sources-2026-10-02.md)。

ios0.1.13 debug binary SHA256
`d8d7bba5fba587cb929bfcfa07e022212398d1b24086d93ce88b8768a8b4686a`。
full 前源码冻结、之后不变；无 storage failure/cleanup，可用约 26GiB 至 24GiB。
没有额外 Chromium/installed3.9/3.13 重跑；catalog/quality/error contract 和历史
MCInstall follow-up 同步，maturity 不变，文档收尾另记录。

文档收尾：118 catalog records、五个 changed Markdown files 的 98 local links
（baseline `adb95d9`）及 whitespace PASS；日志
`/tmp/ios-a02-mux-docs-catalog.log`、`/tmp/ios-a02-mux-docs-links.log`。
源码不变，准备独立聚焦文档提交。

A02/第三类/总体 goal active，第四类未开始，全部 31 任务/子任务 scope 保持。
下一确认的 native loss 是 private NSKeyedArchiver plist 及 backup metadata adapter；
DTX/instruments 原 raw/optional fallback 为另一 behavior contract，未静默改动。
codec integer/typed guards/created outcomes、其他 stream/JSON/callback/binding、
L01/L03/A03/platform/device 条件保留。未操作真实 credentials/设备/Apple，未
push/publish/history rewrite/reference 编辑。

### 大类 3：A02 archive/backup metadata 原始来源与 SQLite 分类交付（2026-10-02）

接续 clean HEAD `c5f338a2355b7567194cc909014e9b540b8bec28`；上一 goal turn 有
MCInstall/usbmux 实现、完整门禁和提交进展，属于 PROGRESS。本轮实现
`dcdda80b2a13e7d3d18f713e20e38be373674343`，五个相关源码文件，首次 full 后
聚焦本地提交；无阻塞条件。

22 个 source adapters 保留原对象：private ArchiveError::Plist 改为 native
plist::Error source；全部 18 Manifest anonymous adapters（12 TryFromIntError、
ParseIntError、2 StripPrefixError、原 identifier validation、ArchiveError、
FromUtf8Error）及 SQLite/crypto/MBDB 三 common helpers。公开 Mobilebackup2Error
四 variants/root APIs/result fields/features 保持；ArchiveError 无 root export/
public signature exposure。Protocol-to-Io 与 fixed phases/Display/correlation/
class 迁移记录。metadata/path/int/UTF8 仍 Proto；depth 保留 ResourceLimit；
crypto IO 原 kind/source/OS code 保留，opaque Crypto marker Unknown。

backup2-manifest 下 SQLite typed codes 供 PermissionDenied/Conflict/ResourceLimit/
Precondition/Protocol/Io 分类，unknown 保留；CannotOpen 不猜 NotFound/permission，
Interrupted 不作 caller cancellation。固定 kind/schema roles 与 native conversion
source 区分，未读 prose/伪造 backend cause。原 Crypto/MBDB 对象保留不等于恢复其
内部早已丢弃的 AES/UTF8 等对象；下一继续内部来源。Native UTF8 bytes/SQL/path 的
source/Debug 为私有数据，context/outer diagnostic Debug safe，无 JSON schema 改动。

Production comparison 仅 adapters/helpers/whitespace，53 个原 map_err calls 保持；
SQL/bounds/depth/path/symlink/version/encryption/Zeroizing/transactions/staging/
replacement/rollback/success 保持，无 deps/binding/protocol/retry/trust/lifecycle 变更。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core | minimal archive21、minimal Manifest27、full backup78 PASS（1 既有 PBKDF2 ignored），terminal0 |
| external / Clippy / comparison | root all21/minimal12 PASS；core/CLI 及 full workspace strict Clippy PASS；production comparison PASS |
| 第11节 full | 首次 `/tmp/ios-a02-backup-sources-full.log` terminal0，1901 Rust/13 Python、0 FAIL、2 既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 边界 | 十一 new core + 一 external；actual archive/plist/cycle/depth、metadata Mode/Size/version、legacy/keybag/IO、symlink UTF8、public list/extract fault/success；actual SQLite corrupt/schema/constraint extended-code/busy/readonly/authorizer/64-byte BLOB limit。typed CannotOpen/Interrupted negative fixtures；非 every code/allocator/fs/architecture/remote cleanup/physical backup |

修复前 corrected 八 initial fixtures 合计 1 PASS/7 FAIL，before-final-archive /
before-final-manifest logs terminal101。更早 broad before.log 17 PASS/8 FAIL 含 fixture
缺 Info.plist/Status.plist，focused debug 也 FAIL；不视作 intended native regression。
补齐 required owned files 后重新复现，valid success PASS；之后增加三 core 与一
external。失败日志保留，未弱化 precondition/gate/lint。准确日志及安全的 test-only
SQLite FFI 约束见[专题证据](validation/backup-metadata-sources-2026-10-02.md)。

ios0.1.13 debug SHA256
`b1f1292e8968cd664b18ecaef8e50f796e7e9268be7cd60c2c5b41ab7a40f5df`。
source full 前冻结、之后不变；无 storage failure/cleanup，约23GiB free。
没有额外 Chromium/installed3.9/3.13 重跑。catalog/quality/error contract 和
historical usbmux follow-up 同步，maturity 不变；文档收尾另记录。

文档收尾：118 catalog records、五个 changed Markdown files 的 100 local links
（baseline `c5f338a`）与 whitespace PASS；日志
`/tmp/ios-a02-backup-sources-docs-catalog.log`、
`/tmp/ios-a02-backup-sources-docs-links.log`。源码不变，准备独立聚焦文档提交。

A02/第三类/总体 goal active，第四类未开始，完整31任务/子任务 scope 保持。
下一 private crypto/MBDB internal native sources，再 host fs/其他 codec/integer/
typed guards/created outcomes、stream/JSON/callback/binding、L01/L03/A03/platform/
device 条件。DTX/instruments 原 raw/optional fallback 未静默改动。未使用真实
backup/credentials/设备/Apple，未 push/publish/history rewrite/reference 编辑。

### 大类 3：A02 备份内部加密与 MBDB 原生来源交付（2026-10-02）

接续 HEAD `df9649e129ac4d8d1969b3dd2a2dbc270e570ae9` 的五个未提交相关源码
变更；上轮已有 native failure 复现和实现进展，本轮继续专项验证及收尾，属于
PROGRESS。实现 `7e5f174169964b713257c037c395312f841dc49c`，首次 full 后按
明确五文件范围本地提交；无阻塞条件。

12 个 private crypto adapters 保留原 AES-KW/cipher/TryFromSlice/TryFromInt
对象，3 个 MBDB adapters 保留 FromUtf8Error/TryFromIntError。CryptoError 六
variants 保持；keybag/class 转换及 unwrap 使用 InvalidData/Proto，opaque cipher/
count 使用 Other/cached Unknown。class-id 原 opaque marker 的转换现在显式
按数据角色 Proto；当前 fixed width guard 后该故障未注入。CBC native IO 原 kind
保持。AES-KW integrity 不证明 password 或 wrapped data 哪个有问题。

现有 optional aes-kw0.2.1 启用 std 以使原 Error 实现 StdError；同时启用该依赖
alloc API，生产仍用原 slice unwrap。版本/lockfile/公开66 feature 名称保持；
crypto-common InvalidLength 已有 Error，无 AES/std 或新依赖。Private MbdbError
tuple 改 Clone/native-source enum，unused private Eq/PartialEq 移除，outer Debug
按原 redaction。Public Mobilebackup2Error 四 variants/root APIs/results 保持；
Display/fixed phase/correlation/私有 variant 与 trait 迁移记录，native UTF8 bytes
为 private data，不进入自动诊断。

Production comparison PASS：仅 native adapters/helper、private error representation、
等价 map closure braces/trailing call commas/whitespace 和 reviewed u16 admission；
Manifest production byte-identical。None 编码 ffff，0..65534 接受，65535 sentinel
guard 和更长 native conversion 均 append 前拒绝；cipher/PBKDF/wrapping/CBC/
Zeroizing/bounds/record order/fileID/success 保持。未改变 wire/binding/retry/lifecycle。
比较 helper 的初次断言未规范化 trailing comma/closure braces，经规范化确认相同，
没有因此改动 production。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core / external | backup minimal/full84/84 PASS，1 既有 PBKDF2 ignored；root all/minimal22/13 PASS，terminal0 |
| Clippy / comparison | core/CLI 及 full workspace strict Clippy PASS；production comparison PASS |
| 第11节 full | 首次 `/tmp/ios-a02-backup-inner-full.log` terminal0，1908 Rust/13 Python、0 FAIL、2 既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 六 new internal、一 external，actual AES-KW/CBC IO/MBDB UTF8/props/field/public legacy list；既有现代 encrypted patch/unback success/rejection 保持。非法 cipher constructor/slice/integer native failure 单独经 common adapter；非 fixed-width/architecture/每个 backend/allocator/rollback/physical/remote cleanup injection |

Corrected before-final 两日志共 0 PASS/4 FAIL，terminal101，证明四个来源丢失。
更早 before logs 为 test-only Option borrow 编译错误，不作 native regression 证据。
首次 post-fix minimal81 PASS/1 FAIL/1 ignored，既有 encrypted fixture 期待已丢弃
password/wrapped prose；改查 Protocol 与 <=32 source 内 native IntegrityCheckFailed，
原 wrong password/no payload/source 存在断言保持。未弱化生产 precondition/gate/lint。
失败日志保留。该失败留下的一个 synthetic fixture 已按 PID/nonce/mtime/expected
files/payload/no symlinks 核实后精确删除，未触及无关临时路径；无 build storage
failure/cleanup。具体日志见[专题证据](validation/backup-inner-sources-2026-10-02.md)。

ios0.1.13 debug SHA256
`ee8bc1e5e2e86fe323e98b840abe6a8b2d99ba002b49bb26c1432b8dfacd6a5d`。
源码 full 前冻结、之后不变；约20GiB 至11GiB free。没有额外 Chromium/installed
3.9/3.13 重跑。catalog/quality/error contract 与历史 metadata follow-up 同步，
maturity 不变；文档收尾单独记录。

文档收尾：118 catalog records，五个 changed Markdown files 的 103 local links
（baseline `df9649e`）与 whitespace PASS；日志
`/tmp/ios-a02-backup-inner-docs-catalog.log`、
`/tmp/ios-a02-backup-inner-docs-links.log`。源码和受测 full gate 不变，准备独立文档提交。

A02/第三类/总体 goal active，第四类未开始，完整31任务与全部子任务 scope 保持。
下一 host filesystem native StripPrefix/missing required IO/Join context 和 archive
integer sources；UID/DTX/instruments 原 optional/raw fallback 另属行为合同，未静默
改变。其他 codec/typed guards/created outcomes、stream/JSON/callback/binding、
L01/L03/A03/platform/device 条件仍待。未使用真实 backup/credentials/设备/Apple，
未 push/publish/history rewrite/reference 编辑。


### 大类 3：A02 主机备份文件与归档整数原生来源交付（2026-10-02）

接续 clean HEAD `2b26398d8c29a5af23e8a77b93c04dfbd41ebe6e`；上一 slice
加密/MBDB 已实现、完整验收及提交，本轮继续实际 source/class 修复，属于 PROGRESS。
实现 `47ed0d5f2649ce4ed65af54ca9b5444246c6e666`，五个相关源码文件，首次 full
后聚焦本地提交；无阻塞条件。

七个原生来源丢失修复：host 五 StripPrefixError adapters、missing-required-file
原 IO、archive unsigned-to-i64 原 TryFromIntError。另一个原已有 JoinError 对象
的 adapter 改用 bounded fixed context/cache，使 panic Unknown（原 generic Io），
cancelled Join 保留 Cancelled；不代表 running worker 终止。缺必需 Info/Manifest/
Status 文件从 Protocol 改 Io/NotFound，保留 native source OS code；outer context
raw_os_error 不是原码，调用者沿 source 查原 IO。Outside-root path 仍 Proto。
固定 phase/Display/correlation/variant/class/get_ref-to-source 迁移有文档。

Private ArchiveError 新增 Integer(native TryFromIntError)，另外三 variants 保留，
没有 root export/public signature exposure；classifier 仅将它加入既有 Protocol arm。
Public Mobilebackup2Error 四 variants/root APIs/results/features 保持。Native path/
panic raw source/Debug 为 private data；context/outer Debug safe，panic hook 未变。
原 created guards/optional UID/M6/raw fallbacks 未静默变化。

Production comparison PASS：仅 adapters/helper、原 NotFound branch payload、新 private
Integer variant、whitespace；host 14 原 map_err calls 保持，Manifest production/
deps/lock/public features/exports byte-identical。Path/symlink/alias/canonicalization/
bounds/incremental/staging/copy/remove/list/statvfs/thread/timer/depth/decoding/success 保持。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core | no-default archive decoder9、host20；minimal Manifest28、full backup87 PASS（1 既有 PBKDF2 ignored），terminal0 |
| external / Clippy / comparison | root all23/no-default12 PASS；core/CLI 和 full workspace strict Clippy PASS；production comparison PASS |
| 第11节 full | 首次 `/tmp/ios-a02-backup-host-full.log` terminal0，1913 Rust/13 Python、0 FAIL、2 既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 四 new internal、一 external；actual path/copy/list、3 required-file/OS code、binary archive/public metadata list/extract rejection/no output、root application reader success/fault。既有 integer boundary/worker IO/panic/timer fixtures 保持；非 copy-parent/cancelled worker/每 OS/allocator/architecture/rollback/running-worker termination/physical/remote cleanup injection |

Corrected before-final 四日志共17 PASS/6 FAIL，每命令 terminal101，证明实际 native
loss 或错误 class。更早 before logs 为 new test lifetime/dictionary key inference
编译错误，不视作 intended regression；修正测试后重复 baseline。既有 unsigned
断言和 private exhaustive negative match 随 Integer 迁移，未弱化 guard/limit/lint。
失败日志保留；new owned UUID fixtures Drop cleanup，未用既有 backup/credentials。
日志与界限见[专题证据](validation/backup-host-sources-2026-10-02.md)。

ios0.1.13 debug SHA256
`c6ac701899c416c8d032e6ce46f6e6f8bd6d1c574b306720ee7f65ab7dabcf1e`。
Source full 前冻结、之后不变。上 slice 后11GiB free，确认 cargo/validate/maturin
全部结束后官方 cargo clean -p ios-core 精确清理38.9GiB generated cache 至48GiB，
日志 `/tmp/ios-a02-backup-host-cache-clean.log`；没有 storage failure/force-rm。
本 full 前45GiB、后29GiB free。无额外 Chromium/installed3.9/3.13 重跑；
catalog/quality/error contract/历史 inner follow-up 同步，maturity 保持，文档收尾另记。

文档收尾：118 catalog records，五个 changed Markdown files 的105 local links
（baseline `2b26398`）与 whitespace PASS；日志
`/tmp/ios-a02-backup-host-docs-catalog.log`、`/tmp/ios-a02-backup-host-docs-links.log`。
源码与 accepted full 不变，准备独立聚焦文档提交。

A02/第三类/总体 goal active，第四类未开始，完整31任务与所有子任务 scope 保持。
下一 readonly audit 已确认 OPACK 三 UTF8 source adapters 与 XPC framing/integer/
deadline source adapters；审查笔记 `/tmp/ios-a02-backup-host-next-codec-audit.log`。
Optional authenticated M6 info/UID/DTX/instruments raw fallback 原兼容行为保持；
其他 codec/typed guards/created outcomes、stream/JSON/callback/binding、L01/L03/A03/
platform/device 条件仍待。未触及真实 backup/credentials/设备/daemon/Apple，
未 push/publish/history rewrite/reference 编辑。

### 大类 3：A02 OPACK/XPC 原生来源交付（2026-10-02）

接续 clean HEAD `2a9814ea47401f72e57b3647fa3641eb2935905e`，本轮实际实现、
验证及提交，属于 PROGRESS；实现 `fcb4cb5dfde7637d74581b72647b97c6b847bd2f`，
十二个相关源码文件，首次 full 后聚焦本地提交。无阻塞条件。

32 个 native erasers 修复：OPACK3、message12、H2 integer4、XpcClient timer3、
RSD slice/timer7、identity UTF8/timer/env3。初始 audit31，再发现 non-Unix env
.ok() 的 VarError loss。原 Utf8/TryFromInt/TryFromSlice/Elapsed/VarError 对象保留；
无 invented backend cause。Public XpcError 四、H2Error 两 variants 保持；private
OPACK InvalidUtf8 增 native payload，classifier 只改其 pattern，32-source bound 保持。

Private body helper 改 IO result，保存 declared u64/native conversion，limit guard 原
消息及 admission/read/allocation order 保持；八 consumers 为 message2/RSD3/H2ready1/
pairing1/DProxy1。Pairing 原 Protocol class 保持，Xpc prose 改 nested Io。DProxy
明确仍在既有 textual event boundary to_string；其他 slice/decoded projection 待审，
不声称 structured observations 完成。Local encode native conversion Protocol 改
InvalidInput；non-Unix NotUnicode 原 collapsed NotFound 改 Protocol，NotPresent 原
NotFound 与 empty guard/success 保持。Variant/Display/correlation/source traversal
迁移、safe outer Debug/private native OS data 记录于[专题证据](validation/xpc-native-sources-2026-10-02.md)。

H2 fatal count conversion 仍 first-error-only poison，返回前无 window writes；
后续 cached Protocol guard 不重建 native cause。其余 windows/frames/buffers/streams
保持。Private shared Tokio mapper 保留真实 Elapsed，原15秒及 separate stage budgets、
connector IO/polling/reconnect factory ownership 保持；identity command cap/drop/
cleanup 保持。未引入新 L01 deadline/retry/remote cleanup acknowledgement。

Production audit PASS：OPACK/H2/message/RSD/pairing adapters、DProxy one adaptation、
XPC helper/public family、原 H2 mapper/classifier/deps/lock/features/root exports。
Client timer consolidation 与 identity extraction/env roles 为 structural review，
非 automated equality。Artifact normalizer 初次遗漏 comments/trailing commas/generic
fn/双 test attributes，修正后 PASS，未据此改 production。Wire/order/bounds/depth/
success 及 optional authenticated M6/UID/DTX/instruments raw fallbacks 保持。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core | no-default lib226、final all-feature XPC94、pairing18、DProxy12 PASS，terminal0 |
| external / Clippy / audit | root all24/minimal13 PASS；core/CLI 与 full workspace strict Clippy PASS；production audit PASS |
| 第11节 full | 首次 `/tmp/ios-a02-xpc-native-full.log` terminal0，1921 Rust/13 Python、0 FAIL、2 既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 七 new core、一 external；actual malformed OPACK/XPC UTF8、private encode/H2 native overflow与poison/no writes、short Elapsed/Drop、hostname/OS-value、public codec success/fault。Actual missing env UUID；NotUnicode 为 owned typed invalid OsString fixture，无 ambient writes。非 actual Windows/macOS identity、public15秒TCPexpiry、narrow-host/每allocator/physical/remote cleanup certification |

Corrected before 七 regression 合计0 PASS/7 FAIL，六命令均 terminal101。
更早 external fixture 误用 private module 触发 E0603，修正为既有 root aliases 后才
复现 baseline，不算 native regression。Post-fix 专项 PASS，最后 env fixture 改用
actual invalid OS data。Existing declared-length/bounds assertions 改查 typed errors，
未弱化 guard/limit/gate/lint；失败日志保留。详情见专题证据。

ios0.1.13 debug SHA256
`f97a7890eab5930433e03801788b6cae73a37986ecfbeec8b421d005d418bc74`。
Source full 前冻结、之后不变，约28GiB 至18GiB free；本 slice 无 storage failure/
cleanup。无额外 Chromium/installed3.9/3.13 重跑。Catalog/quality/error contract 与
历史 backup host follow-up 同步，maturity 保持；文档收尾单独记录。

文档收尾：118 catalog records，五个 changed Markdown files 的107 local links
（baseline `2a9814e`）、capability implementation/maturity/historical evidence 与
whitespace PASS。日志 `/tmp/ios-a02-xpc-native-docs-catalog.log`、
`/tmp/ios-a02-xpc-native-docs-links.log`；源码和 accepted full 不变，准备独立文档提交。

A02/第三类/总体 goal active，第四类未开始，完整31任务与全部子任务 scope 保持。
下一 readonly service audit `/tmp/ios-a02-xpc-native-next-services.log` 确认候选
deadline/address/integer/framing native loss，逐项审查角色和状态再实现；DProxy textual
observations、其他 codec/typed guards/created outcomes、stream/JSON/callback/binding、
L01/L03/A03/platform/device 条件仍待。未触及真实 backup/credentials/设备/daemon/
Apple；未 push/publish/history rewrite/reference 编辑。

### 大类 3：A02 媒体、HID 与抓包原生来源交付（2026-10-03）

接续 clean HEAD `9a16f14c7294db14de4ac334bea247b932984e24`，本轮实现与实际
source/state tests、完整验收，属于 PROGRESS。实现
`5dce4bfcce162b1ebe82b740631e1a0e893fb3d5`，八个相关源码文件，首次 full 后
聚焦本地提交。无阻塞条件。

17 个 native losses 修复：HID2 Elapsed、Display4 address/3 timer/2 integer、
plist frame1 integer、Bluetooth pcapng2 integer、pcap timer/integer/UTF8各1。
最后一个 pcap formatted eraser 超出先前 anonymous search。原 AddrParseError/
TryFromIntError/Elapsed/FromUtf8Error 对象保留，pcap 原 metadata bytes 可沿 source
获取。Fixed context/outer Debug safe，explicit native raw access 为 private data。

HID close 原 Protocol 改 existing Xpc(Io)/Timeout；Display native timer 改 existing
Io，remaining-zero 原 unit Timeout 保持。Local address Protocol 改 InvalidInput，
received config integer Protocol 保持。Pcap IP finder Protocol 改 Timeout，pcap/
pcapng encode Protocol 改 InvalidInput；private plist native adapter direct Unknown
改 InvalidInput。Public Display8/HID5/pcap3/Bluetooth3、private plist3 variants、
signatures/exports/features/classifier 保持；affected variant/Display/correlation/
get_ref-to-source 迁移有[专题证据](validation/media-capture-sources-2026-10-03.md)。

Private 三 framing/pcapng length conversions 等价抽出后保留 native source，
无 giant allocation。BT block 新 checked_add(12) 把 host overflow 作为 source-free
InvalidInput，conversion/writes 前拒绝，不制造 TryFromIntError。原 admitted values/
record order/padding/timestamp/write/flush/unsupported false 与 plist max-before-write
保持。HID reports 只 send、不等待回复；held state 原 send success 后清空。Display
channel-used-before-I/O、fresh connection、stop consume service/request abort before
await、remaining-zero 与 EOF compatibility fallback 保持；repeated local no-op 不
证明 prior remote stop。原 budgets/RTP/caps/counters/optional fields/success 保持，
不引入新 L01 cleanup/reuse/cancellation acknowledgement。

Production comparison PASS：五 HID/十五 Display/三 pcap/二 plist/二 Bluetooth
map calls 保持，仅 native adapters/helper 与等价 conversion extraction；BT guard/
state ordering 单独 review。旧 shared test_util byte-identical，lib.rs 仅新增 private
cfg(test) test_native，public exports/deps/lock/features/classifier 不变。Snapshot
含新 fixture file；日志 `/tmp/ios-a02-media-native-code-audit.log`。

| 检查 | 最终结果 |
| --- | --- |
| 专项 core | all-feature Display17/HID8/pcap10/BT8/plist5、final native60/minimal227 PASS，terminal0 |
| external / Clippy | root all25/minimal13、既有 public Display3 PASS；final core/CLI 与 full workspace strict Clippy PASS |
| 第11节 full | 首次 `/tmp/ios-a02-media-native-full.log` terminal0，1932 Rust/13 Python、0 FAIL、2 既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5 host/22 device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 十 new core、一 external；actual XPC bootstrap/held state/control+stop timers、local UDP expiry/zero、address/config/native bytes/root framed pcap failure；private integer/host-add helpers 与既有 success fixtures。Write gate 在 request bytes 前阻塞，reopen 仅证明 local send/state，非 partial-send reuse/device delivery。非 giant writes/task join/每platform/allocator/architecture/physical/remote cleanup |

修复前两日志共50 PASS/10 FAIL，terminal101；三个 private conversions 在 baseline
fixtures 前等价抽出且仍用旧 erasers，comparison 验证未改变行为。Post-fix 专项
PASS。首次 all-target Clippy 因 helper 置于 integration tests 也 include 的 shared
test_util，crate-root XPC names 不存在，E0433/E0425/E0422 FAIL。改独立 cfg(test)
module 并还原 shared file后 final native/minimal/public/strict Clippy PASS；无 production
guard/gate/lint 弱化，失败日志保留。详见专题证据。

ios0.1.13 debug SHA256
`7b8ce4f8c069a4eb7380c121c8c23b983c696db0fbe1a5a40bd75fa555e063c8`。
Source full 前冻结、之后不变，约18GiB 至17GiB free；无 storage failure/cleanup。
无额外 Chromium/installed3.9/3.13 重跑。Catalog/quality/error contract/历史 XPC
follow-up 同步，maturity 保持；文档收尾另记。

文档收尾：118 catalog records，五个 changed Markdown files 的109 local links
（baseline `9a16f14`）、capability implementation/maturity/historical evidence 与
whitespace PASS。日志 `/tmp/ios-a02-media-native-docs-catalog.log`、
`/tmp/ios-a02-media-native-docs-links.log`；源码与 accepted full 不变，准备独立文档提交。

A02/第三类/总体 goal active，第四类未开始，完整31任务/子任务 scope 保持。
下一 readonly search 为28 service files/66 anonymous candidates，非66 confirmed
native losses；formatted/.ok()/created/optional guards另审。日志
`/tmp/ios-a02-media-native-next-services.log`、
`/tmp/ios-a02-media-native-next-timeout-audit.log`。WebInspector quiescence/startup
compatibility 使用 named Timeout 正常完成，保留 Elapsed 时必须保持 issuer语义，
不能把 every IO TimedOut 都作为正常完成。Other services/DProxy text/codec/typed
guards/created outcomes/stream/JSON/callback/binding 与 L01/L03/A03/platform/device
条件仍待。未使用真实 credentials/device/daemon/Apple；无 push/publish/history
rewrite/reference 编辑。

### 大类 3：A02 发现服务原生来源与期限发行方交付（2026-10-03）

接续 HEAD `8a08f22742bf61d016d812c8a5422af14d93968a`，本轮实施及专项/完整
验收属于 PROGRESS，无阻塞条件。实现
`89bcd3f81f6e0617e8a1526f3147feb5e7197876`，十二个相关源码文件，本地聚焦提交。

22 native losses 修复：WebInspector14（Client3/Inspector3/Automation5 Elapsed、
Page/PID2 ParseInt、frame1 TryInt），Companion6（send/roundtrip/listener/stop4
Elapsed、negative/u16 port2 TryInt），Notification2 Elapsed。PID formatted eraser、
Companion match arm/.ok() 超出先前 anonymous search。原 native 对象/固定 context
保留在 existing Io，timer Timeout、received conversion Protocol、local frame
conversion InvalidInput；returned variant/Display/source/correlation 迁移见
[专题证据](validation/discovery-native-sources-2026-10-03.md)。

Public WebInspector6/Companion4/Notification5 variants 保持，新增两 error
`timeout_duration()` 方法。Private issuer payload 同时保留真实 Elapsed 和旧
Timeout adapter 报告的 Duration，getter 最多32 typed sources、不格式化，不把
generic IO TimedOut/standalone Elapsed 当作本服务期限。None 不证明不是 timeout。
Startup write 报告 original budget，response/event 可报告 remaining phase budget。
Web listing quiescence、CLI discovery2/CDP2 消费 own expiry 语义保持；底层 IO
timeout 仍报错。Legacy start 的 response own timeout/closed stream 仍故意 Disabled/
丢 source；blocked identifier-write 直接返回 native Io，preserving start 保留 source。
Created remaining-zero markers不伪造 Elapsed。

Companion listener partial read 仍 finished-before-return、不resume，stored pending
future ownership保持；forward stop 仍成功 response 后才设置 stopped/cache，Drop
best effort保持。Port unsigned precedence/type/zero/range保持，negative conversion
真实 source可观测。Notification observe send-success后更新、ProxyDeath/zero保持。
Web max-before-write、explicit PageID precedence、PID delimiter/success保持。
Private frame converter baseline前等价抽出，直接 helper测overflow无 giant allocation，
现有16MiB cap保持。Protocol/budgets/retries/wire/state/cleanup未更改，不引入L01
partial-operation reuse、remote cleanup或task join保证。

Production comparison PASS：Inspector3/Automation5/model1/Notification2/client5/
wire2/Companion9 map_err保留；native adapters/helpers、frame等价抽出、own-deadline
predicate以外相等；port rewrite/issuer/getter/listener arm单独 review。CLI仅4
consumer branches及unused CDP import/rustfmt。Cargo/lock/core root/classifier/
test_native byte-identical。日志 `/tmp/ios-a02-discovery-native-code-audit.log`。

| 检查 | 最终结果 |
| --- | --- |
| core 专项 | all-feature Web38/Companion16/Notification10；对应 no-default service features38/16/10；final native12 PASS，terminal0 |
| external / CLI / Clippy | Web12、root all26/minimal13、CLI Web39；final core/CLI和 full workspace strict Clippy PASS |
| 第11节 full | 首次 `/tmp/ios-a02-discovery-native-full.log` terminal0，1945 Rust/13 Python、0 FAIL、2既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5host/22device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 十二 new core、一 external：actual owned duplex/pending timers/PID/page/port、typed bounded cycle/foreign IO fixture、partial Companion terminal state、forward failure state/Notification observations和exhaustive root consumers。Automation later attach stages仅source review；private frame helper直接转换。非actual OS timeout/giant writes/partial-write reuse/task join/platform/physical/remote cleanup |

Corrected baseline日志共0 PASS/11 FAIL，terminal101；initial E0283 Dictionary
fixture编译失败先修正，不计native regression。Post-fix专项PASS。First external
Web11 PASS/1 FAIL因新断言误要求exact20ms，改为positive remaining<=20ms并保留
late完整帧断言；原budgets不变。First strict Clippy发现unused CDP import/test IO
constructor，修正后finalPASS；未弱化production guard/gate/lint。原失败日志保留，
完整命令及边界见专题证据。

ios0.1.13 debug SHA256
`97defac193aecb4303b51abfa72587a46a40695626363ce08cf789600bbae7db`。
Source full前冻结、之后与audit snapshot一致；full前后约15GiB free，无storage
failure/cleanup，无额外Chromium/installed3.9/3.13重跑。Error/public contract、
quality/catalog与historical media follow-up同步，maturity保持；文档收尾另记。

文档收尾：118 catalog records，六个 changed Markdown files 的136 local links
（baseline `8a08f22`）、capability implementation/maturity/historical evidence及
whitespace PASS。日志 `/tmp/ios-a02-discovery-native-docs-catalog.log`、
`/tmp/ios-a02-discovery-native-docs-links.log`；七文档文件独立提交，源码和accepted
full保持不变。

A02/第三类/总体 goal active，第四类未开始，完整31任务/全部子任务scope保持。
Readonly next scan 21service files/47 anonymous candidates，非47confirmed native
losses。日志 `/tmp/ios-a02-discovery-native-next-services.log`、
`/tmp/ios-a02-discovery-native-next-consumers.log`；CrashReport watch parse retry
消费own parser failure，后续保留source须保持issuer而非retry every Protocol/IO。
其他services/DProxy text/codec/typed guards/created outcomes/stream/JSON/callback/
bindings与L01/L03/A03/platform/device条件仍待。未使用真实credentials/device/
daemon/Apple；无push/publish/history rewrite/reference编辑。

### 大类 3：A02 崩溃报告与 OS Trace 原生来源交付（2026-10-03）

接续 clean HEAD `d993ecf4590547de8f200d9cb4bdb4e3a84504d7`，本轮实现及实际
专项/完整验收属于 PROGRESS，无阻塞条件。实现
`1c8174275aadff24a1cb3001504e7451efc95fcf`，四个源码文件，聚焦本地提交。

22 native adapter sites：CrashReport9（5 Elapsed、2 UTF8、2 JSON），OS Trace12
（2 Elapsed、5 integer、4 IO、1 regex），CLI PID1 integer。Original对象通过
existing Io/private source保留。Timer为Timeout class、received report/trace decoding
保持Protocol、local request/PID/regex为InvalidInput。Archive IO保留原kind/native
OS chain，不再Protocol prose；host header UnexpectedEof为IO，不推断disconnect。
Native tar IO也可从Protocol改IO；created format/path/link/limit guards保持Protocol。
Public Crash5/Trace4 variants、existing signatures/fields/root/features/deps/lock/
classifier保持；returned variant/Display/source/kind/correlation迁移见
[专题证据](validation/diagnostic-native-sources-2026-10-03.md)。

新增 MessageFilterOptions.compile_preserving_errors，Core/CLI选择typed方法。
Legacy compile保持String signature/formatting与故意source loss；共享旧bounded
filter实现，limits/case/Unicode/predicates/success保持。Term count/length为source-free
Input guard、不伪造RegexError。CLI PID/regex从Unknown改Input，preflight/connect/
signal顺序、exit/output/envelope shapes保持；private PID wrapper修复std IO source
forwarding会跳过bare TryInt问题，Debug/Display固定。

Crash watch private parser marker保留UTF8/JSON source，最多32 typed sources判定
own parse failures与旧Protocol parser guards，不依赖Display或everyProtocolClass。
原3 parse retries、pending/known/snapshots/poll/signature/yield/terminal规则保持。
Admission timeout可正常结束，in-flight expiry仍错误；checked-add overflow保持
source-free unitTimeout。Owned AFC peer验证initial/poll/read三个timer和4次完整
malformed JSON file opens/reads/closes，最后source可观测，client丢弃后peer EOF/join。
仅local fixture cleanup，非device file-close/cancel acknowledgement或partial reuse。

OS request converter在baseline前等价抽出；仍conversion在8MiB guard/write前，
helper直接测overflow无giant allocation。4received usize conversions在64bit仅source
review，非32bit failure injection。Archive header/checksum两passes/native extraction
OS fault与owned file cleanup验证；trace EOF/framing/caps/archive paths/links/types/
staging/install/modes/cleanup保持。不新增L01 remote release/retry/invalidation保证。

Production comparison PASS：Crash9maps；sharedfilter2guard/1regexcallback normalized
body等于旧compiler；剩余Os13mapping sites包括typed consumer/既有move-file；CLI5
maps normalize后body不变，private source helpers单独review。Cargo/lock/root/classifier/
test_native byte-identical。日志 `/tmp/ios-a02-diagnostic-native-code-audit.log`。

| 检查 | 最终结果 |
| --- | --- |
| core 专项 | final native9、all-feature Crash28/Trace31、对应 no-default28/31 PASS，terminal0 |
| external / CLI / Clippy | root all27/minimal13、既有OS target5（含shared fixtures）、final CLI OS11；core/CLI/full workspace strict Clippy PASS |
| 第11节 full | 首次 `/tmp/ios-a02-diagnostic-native-full.log` terminal0，1956 Rust/13 Python、0 FAIL、2既有 ignored |
| 其余 gates | fmt/check/MSRV1.80、66 features/4 groups、7+7 doctests/3 snippets、C ABI/owned malformed/checked list、3.12 wheel/sdist、53 tools、118 help、38 raw、5host/22device NOT_RUN、15 CLI/3 backup PASS |
| 主机边界 | 九new core、一external、一CLI；actual owned AFC/duplex/parser/regex/archive/OS sources、bounded cycle/fixed Debug、retry limit与local peer/file cleanup。非every OS/backend/allocator/giant writes/partial operation reuse/physical/remote cleanup |

Genuine before-fix service/public logs共0 PASS/9 FAIL，terminal101；CLI原baseline
与first post-fix使用无效level all，提前失败，不计native-loss baseline。改actual
empty-default fixture后发现Input正确但standard source中缺TryInt（10PASS/1FAIL），
修private source wrapper后finalCLI PASS，不弱化断言。First public post-fix E0433为
新增ErrorClass断言未限定路径，修fixture后PASS。Audit原误估CLI3maps，实际5maps
验证。Original failure logs保留，production guards/budgets/gates/lints未弱化。

ios0.1.13 debug SHA256
`9778f708eb92b42c85f1b61a29dca79c14ccdd8b0617d6ca3a89ccc4059295b5`。
Source full前冻结、之后与audit snapshot一致；full前后约13GiB free，无storage
failure/cleanup，无额外Chromium/installed3.9/3.13重跑。Error/public contract、
quality/catalog与historical discovery follow-up同步，maturity保持；文档收尾另记。

文档收尾：118 catalog records，六个 changed Markdown files 的139 local links
（baseline `d993ecf`）、capability implementation/maturity/historical evidence及
whitespace PASS。日志 `/tmp/ios-a02-diagnostic-native-docs-catalog.log`、
`/tmp/ios-a02-diagnostic-native-docs-links.log`；七文档文件独立提交，源码和accepted
full保持不变。

A02/第三类/总体goal active，第四类未开始，完整31任务/全部子任务scope保持。
Readonly next scan19service files/33 anonymous candidates，非33confirmed native
losses；formatted/match/.ok()/created guards另审。日志
`/tmp/ios-a02-diagnostic-native-next-services.log`、
`/tmp/ios-a02-diagnostic-native-next-consumers.log`。Cryptex/activation/InstallCoordination/
pasteboard deadlines须保留old phase/Duration/seconds与state，区分actual Elapsed和
created admission/sleep markers；其他native/typed guards/DProxy text/stream/JSON/
callback/bindings与L01/L03/A03/platform/device仍待。无真实diagnostics/backup/
credentials/device/daemon/Apple，无push/publish/history rewrite/reference编辑。

### 大类 3：A02 服务期限、原生来源与 promised-size 上限交付（2026-10-03）

从 clean `3144ca5d0a0fd90091bbb17d5feddd8430bc1edc` 恢复，分支
`feat/device-service-updates-20260927`。实现提交
`bd6ee91459fd34b2b2f42f419f12b90dccd259da`，九个源码文件；聚焦本地交付，
不推送/发布。Linux x86_64、Rust 1.97.1/MSRV 1.80.1、Python 3.12.13。

保留21处实际原生来源：Cryptex四个Elapsed、assets三个reqwest及一个cache
rename IO（八处）；MobileActivation两个Elapsed；InstallCoordination connect
一个Elapsed和一个原始CoreError；Pasteboard/client/subscription/monitor七个
Elapsed及TryFromIntError/uuid::Error（九处）。已有Io/Xpc(Io)承载来源，
四个public enum仍11/5/3/7 variants与原fields穷举。新增
`CryptexError::timeout_phase()`、MobileActivation/InstallCoordination
`timeout_duration()`、`PasteboardError::timeout_seconds()`，保留旧Timeout的
phase/Duration/whole seconds及as_secs截断。最多32个typed sources，只认自己的
private issuer或legacy Timeout；generic IO TimedOut/unissued Elapsed返回None，
不format，None不排除timeout，也不证明retry/cleanup完成。
[专题证据](validation/service-deadline-native-sources-2026-10-03.md)。

Native timers保持timeout，received pasteboard integer/UUID保持protocol。
Cryptex HTTP timeout/builder从opaque Download变为timeout/invalid_input；native
cache IO保留kind/OS source，可能从Protocol变为io；InstallCoordination connect
保留CoreError原class。Returned variants/Display/immediate get_ref/IO kinds与safe
correlation可能改变；typed class/source/issuer getters迁移，native显式访问遵循
redaction边界。Created activation admission/poll/sleep和InstallCoordination
query/select deadlines仍source-free legacy Timeout，不伪造Elapsed。

私有budget/connect/fetch/builder/publish helpers在before baseline前等价提取，
公开Cryptex routine/preflight30s、install600s及HTTP connect/total30/600s保持，
used=true之后下一routine重连规则保持；activation绝对期限和initial unsupported
fallback、canonical metadata gates、InstallCoordination query unusable/late-reply
规则保持。Pasteboard公开10s期限、五条timeout owner/closed/idempotent unsubscribe
路径、monitor initial-baseline/read fail-closed与无期限idle等待保持。Timed future
drop后take XPC owner顺序、其他错误原reuse策略与best-effort Drop保持，不扩展L01
cleanup/retry/remote acknowledgement保证。

一处显式guard修复：非负promised size以u64完整比较data limit，关闭32bit上
usize conversion失败后跳过上限的路径；negative/over-limit仍source-free
Protocol/Limit。Host覆盖exact limit、超限与i64::MAX，32bit仅source review，
无32bit runtime claim。Cryptex fixed-four paths Vec-to-array Err不是StdError；
上轮next-consumers称TryFromSlice不正确，该guard保持。

生产比较：提取baseline后named native callback normalization、五条pasteboard
owner/state bodies、cache remove/concurrent-load/fallback顺序、一个full-width
promise条件PASS。MobileActivation send_command normalization包含两个未改的
frame adapters；四enum/macro family exact；Cargo/lock/root/classifier byte-identical。
Shared XPC fixture只扩展test cfg gates。完整diff另审提取/固定context；日志
`/tmp/ios-a02-service-deadline-code-audit.log`，source patch SHA256
`1c2a6d64dab344e1e681c616ae117d47c6e51f45f784d40b7a525ec262d497de`。

| 检查 | 实际结果 |
| --- | --- |
| native_source group | 19 PASS：16新增internal与3既有；after-final.log terminal0 |
| all-feature Cryptex/activation/InstallCoordination/pasteboard | 20/33/28/30 PASS |
| 对应单个no-default feature | 20/29/28/30 PASS；activation另四TLS fixtures需其他features |
| 外部public_contract all/no-default | 28/13 PASS；每服务minimum getter另1 PASS |
| core strict Clippy | clippy-final.log PASS，terminal0 |
| 第11节full | 首次full.log PASS terminal0，1973 Rust/13 Python、0 FAIL、2既有ignored |

新增测试触发actual owned H2/XPC stalled operations、activation command/session
期限、loopback HTTP send/chunk timeout及builder fault、cache rename native OS
error/有效竞争cache fallback、received conversions、五条pasteboard owner路径、
monitor baseline/read状态及cyclic/panicking Display/unissued getter边界。
Cryptex connect/preflight和InstallCoordination connect用相同私有边界/owned
pending/Core futures，公开设备入口placement仅source review。Invoke/install
使用真实XPC client和synthetic on-disk assets；local EOF/join/owned file Drop只
证明fixture cleanup，不证明partial-write reuse、设备cancel/stop或remote cleanup。

Before实际13新增native tests FAIL、3既有PASS，terminal101；首次fixture compile
E0283（String keys）/E0716（borrowed path temporary）仅fixture修正后再跑genuine
baseline，日志before-compile/before保留。首次activation组32PASS/1FAIL：迁移
断言错误要求caller10ms等于internally-started absolute budget；改为>0且<=10ms
并保留actual Elapsed检查，生产budget/fallback不改；final33PASS。日志均采用
`/tmp/ios-a02-service-deadline-`前缀，专题列完整commands/records；guard/lint未削弱。

Full覆盖fmt/check/workspace strict Clippy/MSRV、66feature builds/四groups、
7+7doctests/三snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53tools、118help、38raw、5host workflows/22device NOT_RUN、15CLI/
3backup contracts。Tested ios0.1.13 debug SHA256
`735b7a46d62d9b705062febd25c0302cc1a870afa092194b93ee0756fb9f9172`。
Full前后源码冻结；磁盘约8.1/7.3GiB，无storage failure或cleanup；无额外
Chromium/installed3.9/3.13、external DDI download/real TSS/pairing/credentials/
device/daemon/Apple。Error/public/quality/catalog及historical diagnostic
follow-up同步，implementation/maturity和历史evidence保持；文档另提交。

文档收尾：118capability records、六个changed Markdown的142local links（baseline
3144ca5）、implementation/maturity/历史evidence保持、单个new host evidence与
tested commit一致、whitespace PASS。docs-catalog/docs-links.log同上述前缀；
七文档文件独立提交，accepted源码/full保持不变。

A02/第三类/总体goal active，第四类未开始，完整31任务/全部子任务scope保持。
扩大readonly扫描90candidates/28files，含36anonymous/22files，包括test branches/
deliberate optional/string observers，不是confirmed losses。next-services、
next-anonymous、next-dproxy及next-consumers日志同前缀；下一步核对ImageMounter/
restore HTTP/TSS/ZIP native角色与原budget/proxy/limit/guards、MobileBackup2
四个Unback/Extract operation/finish deadline及state/cleanup规则，再处理AFC/
DTX/integer/DProxy text/typed guards/created outcomes/stream/JSON/callback/bindings
与L01/L03/A03/platform/device条件。没有删除任务或maturity升级。

### 大类 3：A02 DDI 下载、ZIP 与 TSS 原始来源交付（2026-10-03）

从clean `818dd69704c10006783c1544308be6aa52af8be1`继续，分支
`feat/device-service-updates-20260927`。实现
`120b07cbfeba1665af62aad0bdc8ad34d47104f6`，七源码文件，已聚焦本地提交。
Linux x86_64、Rust1.97.1/MSRV1.80.1、Python3.12.13；上轮属于已验进展。

保留34个原始native boundaries：DDI download/cache/extract的17 IO、6reqwest、
4ZipError、1JoinError（28）；TSS的2plist、3reqwest、1UTF-8（6）。两个download
GET边界共享等价private request helper，最终27+6callbacks。Native IO/ZIP inner
IO保持kind/OS cause；received ZIP/UTF-8/plist为protocol，local serialization/
HTTP builder为invalid_input，HTTP expiry为timeout，host JoinError cancellation
为cancelled、panic保持unknown；其余HTTP保留自身category含unknown。Original
Download/Tss可能转为已有Io；variant/Display/get_ref/kind/class/safe correlation
有迁移，native显式访问受redaction约束，不伪造source-free guard的native来源。
[专题证据](validation/image-assets-native-sources-2026-10-03.md)。

Public ImageMounterError仍六variants/原fields穷举，无新public signatures/getters；
root exports/features/Cargo/lock/classifier/protocol/manifest保持。Private request/
body/builder/serialization/extraction-result/file/body-copy helpers在旧adapter
baseline前提取。Headers/body/URL/build constants、version/cache/fallback/abort、
warn-only cache write、HTTPS_PROXY→HTTP_PROXY precedence保持；download拒绝bad
proxy、TSS permissive忽略bad proxy策略保持，tests不改process env。TSS30/120s与
16MiB cap保持，download builder策略保持，无新增total/resource/output bound。

Content-type先于spool创建、ZIP magic/path filtering/extraction顺序、personalized
load/build校验与success后删除tmp ZIP保持。Failure时partial spool/extracted file
仍可能留下、drop awaiter不保证停止blocking work；Source保留不证明retry/rollback/
L01 cleanup。整份提取download/TSS源码只normalize27+6nativecallbacks后相等，
其余bodies/guards/policies完整比较；extraction/private wrappers另审，不用happy
path代替whole-body。上述public/schema/deps/root/protocol/manifest/shared fixture
byte-identical；日志code-audit/source-diff同`/tmp/ios-a02-image-assets-`前缀，
source diff SHA256
`2d82729e2acc6ce02951fde62ab6ce39a2973cc22db5497c8b90df3971fa4fcd`。

| 检查 | 实际结果 |
| --- | --- |
| ImageMounter all-feature/独立minimum feature | 各30 PASS；after-final/minimal.log terminal0 |
| external public_contract/对应minimum consumer | 29/1 PASS；public/public-minimal.log terminal0 |
| core strict Clippy | clippy.log terminal0 |
| 第11节full | 首次full.log terminal0，1991 Rust/13 Python、0FAIL、2既有ignored |

新增17internal与1external，owned cache/OS/ZIP malformed/encrypted/CRC、loopback
HTTP request/body/spool timers、真实local TSS headers/body成功、UID/received
UTF-8/plist、host task abort/panic与exhaustive public cache path覆盖。Writer
write/flush和ZIP reader用captured owned OS create error注入generic fixture，
非实际disk-full/write/flush/all-backend syscall验收；其他file/cache/extract
有真实OS路径失败。HTTP peers bounded join允许read IO error，非逐peer EOF证明；
owned目录Drop清理仅fixture行为。Public external/device入口placement仅source
review，无external DDI/real TSS/pairing/mount/credentials/device/daemon/Apple。

基线诚实区分：before-final15PASS/13FAIL中的一个ZIP坏夹具在open时已失败，
排除native-loss证据，另外12为genuine；初始before同夹具、12total FAIL。
before-public0PASS/1FAIL为真实external native loss。首after26PASS/2FAIL是ZIP
fixture和未进入native plist parsing的TSS样本；修为encrypted-entry与有XML
declaration/incomplete-dictionary后通过。Targeted before-corrected-zip/plist
临时仅恢复四旧ZIP和一个旧plist adapter：2PASS/3genuine native FAIL，非全旧源码
rerun；之后全部恢复fixed source，再final相关/full。Logs保持、无compile failure、
无削弱guard/budget/lint。专题列exact scopes，不把夹具错误或重复计数当新增证据。

Full包含fmt/check/workspace strict Clippy/MSRV、66features/四groups、7+7doctests/
三snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/install、53tools/
118help/38raw、5host workflows/22device NOT_RUN、15CLI/3backup contracts。
Tested ios0.1.13 debug SHA256
`828fb95e0be4628f64ee6723e1016a3da771c973a6363354870ab3b4ad623600`。
Full前后源码冻结，磁盘约6.5/6.1GiB，无storage failure/cleanup；无额外Chromium/
installed3.9/3.13。Error/public/quality/catalog与historical service-deadline
follow-up同步，CHANGELOG Unreleased登记DDI/TSS，并补近两轮getters/promise与
typed filter/diagnostic迁移条目；implementation/maturity和history evidence保持。

文档收尾：118capability records、七个changed Markdown的150local links（baseline
818dd69）、implementation/maturity/历史evidence保持、单个host entry与tested
source commit一致、whitespace PASS；docs-catalog/docs-links.log同前缀，八文档
文件独立提交，accepted源码/full不变。

A02/第三类/总体goal active，第四类未开始，完整31任务/全部子任务scope不变。
扩大readonly scan剩59candidates/26files，含test/intentional optional/text observers，
非confirmed losses；next-services/next-backup/next-conversions日志同前缀。
下一步审MobileBackup2 Unback/Extract四operation/finish deadlines的primary
error/disconnect-state规则，及restore TSS/preflight、AFC/DTX/file/symbol/app/test
conversions；DProxy text/typed guards/created outcomes/stream/JSON/callback/bindings
与L01/L03/A03/platform/device仍待。无push/publish/history rewrite/reference编辑。

### 大类 3：A02 MobileBackup2 期限与主错误保留交付（2026-10-03）

从 clean `01a72928930d6e2ba02582a29d09434b7274dae9` 继续，分支
`feat/device-service-updates-20260927`。上轮属于已验进展。本轮实现
`145d0c41f839a22e757e4aef616885b5303f4d6f`，六个源码文件，已聚焦本地提交。
Linux x86_64、Rust 1.97.1/MSRV 1.80.1、Python 3.12.13。
[专题证据](validation/backup-deadline-native-sources-2026-10-03.md)。

保留六个原始 native 边界：Unback/Extract 的四个 operation/finish Elapsed，
received path 的一个 FromUtf8Error，transfer bytes 的一个 TryFromIntError。
两个入口在基线前提取为等价 private dispatch，最终共享两个 timer callbacks。
原有 Protocol 超时改为 Io TimedOut，class 为 timeout；UTF-8/count 改为
Io InvalidData、class 仍 protocol。count 转换在本 64-bit 主机无法失败，只作
source review，不制造 overflow/native failure 或宣称其他架构运行验收。

新增 `Mobilebackup2Error::device_operation_timeout()`，至多读取 32 个 typed
sources、无需格式化；返回 private-field `DeviceOperationTimeout`。operation/
phase/budget getters 使用两个新 non-exhaustive enums 和原 configured total
Duration。generic IO TimedOut、foreign Elapsed、prose/cycle 不构成本发行方
证据；None 不代表没有 timeout。原四个公开错误 variants/fields、facade fields/
signatures、root/features/deps/classifier 保持。

修复真实主错误丢失：result 留在 timed disconnect future 外，收尾超时时
不再随该 future 一起销毁。最终标准 source/class 是收尾 Elapsed/timeout，
`operation_error()` 独立借用原 Mobilebackup2Error/native sources，包括先前
operation Elapsed。两个错误分支明确分开，不宣称一条线性 source chain 可含两因。
若 operation 成功而 disconnect 超时，仍无成功 Content 返回；普通 best-effort
disconnect failure 的 suppression/warning 和原 result 保持。

公共 300s 常量和同一个 absolute deadline 保持。完整 inner Unback/Extract
bodies、host validation/layout 在 negotiation 前的顺序、请求字段/Unicode/optional
omission、Hello/version/transfer loop/counters/state reset/guards 保持；其他
backup/restore/info/list/password finish_session 完整比较为原样。整份 production
protocol 和 transfer helper tail 仅分别 normalize 一个 native callback 后相等；
dispatch extraction 与有意 ownership 修复另审。Cargo/lock/root/classifier、
DeviceLink、host_fs/manifest/crypto/MBDB/shared fixture byte-identical。
code-audit-final/source-diff 等日志同 `/tmp/ios-a02-backup-deadline-` 前缀；
source diff SHA256
`e50dfedd077ef40c0cb989a67a01cc32e5deefc3cbc6c20ffa43da21fe79b749`。

| 检查 | 实际结果 |
| --- | --- |
| backup module all-feature/no-default | 96 PASS/1 既有 ignored；51 PASS；after-final/minimal.log terminal 0 |
| external public_contract all/no-default | 30/14 PASS；public/public-minimal.log terminal 0 |
| core strict Clippy | clippy.log terminal 0 |
| 第 11 节 full | 首次 full.log terminal 0，2001 Rust/13 Python、0 FAIL、2 既有 ignored |

新增九个 internal 和一个 external tests。Owned duplex/temporary layout 实际
Tokio timer、gated pending disconnect、native peer EOF/protocol failure、成功
Content 后 cleanup expiry、double Elapsed、zero budget、received UTF-8 有证据。
Peer/client futures 在 3s 内 join、ready disconnect 在 1s 内读出，fixture 目录
Drop 清理；gated write 非 OS write timeout。External 覆盖 API shape/borrow/
Send+Sync、穷举 family、有界无格式化 issuer lookup，未运行公共五分钟 timer；
公共 placement 仅 source review，未冒充真机/Apple/远端 cleanup 验收。

两阶段负控制明确区分：等价 dispatch 与新 observation types 已存在、旧 adapters
尚未发行证据的 before.log 是 87 PASS/9 genuine FAIL/1 ignored；仅保留 native
sources、仍将 result 移入 finish 的 before-primary.log 是 92 PASS/4 genuine
FAIL/1 ignored。四项分别丢失 Unback/Extract protocol、native EOF 和先前 timeout；
ownership 修复后全过。没有 compile failure、坏夹具、guard/lint/budget 削弱，
失败日志保留，不把同一组失败重复计作新增 regressions。

Full 含 fmt/check/workspace strict Clippy/MSRV、66 features/四 groups、7+7
doctests/三 snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53 tools/118 help/38 raw、5 host workflows/22 device NOT_RUN、15 CLI/
3 backup contracts。Tested ios 0.1.13 debug SHA256
`a4f9ab1280248dfdf85145fd7b45dcb781c544fdef6ecebace46d48e06bba826`。
源码在 full 前后冻结；磁盘约 5.7/4.4 GiB，无 storage failure/cleanup；无额外
Chromium/installed 3.9/3.13。文档收尾记录另行审查并提交，不改 accepted 源码。

文档收尾：118 capability records、七个 changed Markdown 的 153 local links
（baseline 01a7292）、implementation/maturity/历史 evidence 保持、单个 host
entry 与 tested source commit 一致、whitespace PASS；docs-catalog/docs-links
日志同前缀。CHANGELOG/error/public/quality/catalog/roadmap/新专题与前专题
follow-up 共八个文件独立提交，accepted 源码/full 不变。

A02/第三类/总体 goal active，第四类未开始，全部 31 任务及子任务 scope 保持。
本次主错误修复不结束 L01：blocking work 可能在 awaiter drop 后继续、partial
file/stream effects、success-content recovery、client reuse、remote cleanup/
cancellation/retry 仍待独立工作。下一确认项是 restore TSS/IPSW/preflight 与
H2/XPC control/bootstrap 的 native adapters，read-only next-restore/context
日志同前缀；其他 AFC/DTX/file/symbol/app/test conversions、DProxy text、typed
guards/created outcomes/stream/JSON/callback/binding 与 L01/L03/A03/platform/
device 条件继续。无设备/credentials/daemon/Apple 操作、推送/发布、历史改写或
reference checkout 修改，capability maturity 不升级。

### 大类 3：A02 restore H2/XPC、IPSW 与 TSS 来源交付（2026-10-03）

从 clean `a5bb2f409eb22f73ad83dee4a6b13121a2bdcb68` 继续，分支
`feat/device-service-updates-20260927`。上轮属于已验进展。实现
`d1a210f2ff466ae8f9e5f281e87ebb69823092fa`，九个源码文件，已聚焦本地提交。
Linux x86_64、Rust 1.97.1/MSRV 1.80.1、Python 3.12.13。
[专题证据](validation/restore-native-sources-2026-10-03.md)。

保留 25 个原始 returned-error adapters：Restore control/bootstrap 的 7 H2Error
和 6 XpcError（13）；IPSW/preflight 的 3 ZipError、1 plist、1 IO、1 TryFromIntError
（6）；TSS 的 3 reqwest、2 plist、1 Utf8Error（6）。均通过已有 RestoreError::Io
和 fixed private context 保留原对象；native IO/ZIP/H2 inner IO 的 kind/category/
OS cause 保持，received H2/TSS codec 为 protocol，XPC 保留原 class（含 input/
opaque unknown）；local ZIP format/password/manifest/TSS serialization 为 input，
HTTP builder/expiry 为 input/timeout，其余 HTTP 保持原 class（含 unknown）。
原 Protocol variant/Display/get_ref/kind/class/safe correlation 有迁移，explicit
source 受 redaction 约束；source-free direct guards 不制造 native OS cause。

公开 family 仍三个 variants/原 fields 穷举，无新 public getter/enum field/
signature。Root/feature/deps/lock/classifier 保持；private TSS builder 和 control
buffer helper 在旧 adapter 基线前等价提取。整份 production mod/preflight/TSS
仅 normalize 13/6/6 个 native callbacks 后相等，每个 final callback 明确走 native
helper；其他 bodies/guards/policies 完整比较，extraction/kind wrappers 另审。
Cargo/root/classifier/shared native fixture/H2/XPC codecs byte-identical。
code-audit-final/source-diff/baseline/original 等日志同 `/tmp/ios-a02-restore-native-`
前缀，source diff SHA256
`8a52dcf3e4502ccca04836275672a40e375b249be6c6a68866d4807708610bd8`。

Wire flags/streams/bootstrap、request ID 在成功 write 后递增、control filtering/
buffering、preflight consume-one connection 保持。TSS 30/120s、4/16 MiB caps、
headers/URL/POST/status/nonce/batch fallback 保持；IPSW identity/variant、100k entries/
256 identities/8 MiB manifest/64 MiB firmware/depth64、path/duplicate/size guards
及 256 KiB chunk iterator 保持。无新 total/cleanup deadline、archive allocation
bound、blocking-work stop、fail-closed/retry/rollback/reuse/remote cleanup 保证。

| 检查 | 实际结果 |
| --- | --- |
| restore all-feature/独立 no-default restore | 各 44 PASS；after-final/minimal.log terminal 0 |
| external public_contract all/restore minimum | 31/15 PASS；public/public-minimal.log terminal 0 |
| core strict Clippy | clippy.log terminal 0 |
| 第 11 节 full | 首次 full.log terminal 0，2017 Rust/13 Python、0 FAIL、2 既有 ignored |

新增 15 internal 和 1 external tests。Scripted owned AsyncRead/Write 输入有效
SETTINGS/control，六个 write 位置覆盖 connect 与所有 bootstrap write/open；
write fault 用 captured owned OS file-open error 注入，非实际传输 write syscall。
Exhausted scripted input 导致真实 read_exact UnexpectedEof，不称真实 peer EOF。
Malformed settings/control UTF-8、ID failure state 和 exact private helper 的
bounded XPC buffer guard 有证据；未冒充完整 live 64 MiB response。

Synthetic IPSW malformed/encrypted/CRC/manifest、archive open/entry scan native
IO、actual plist/UTF-8/UID codecs 与 loopback HTTP request/body timer、builder/
endpoint 有证据。Reader/seek 用实际 captured OS file-open error 注入，非真实
IPSW FS read/seek 或所有 backend。Request 与三处 fixed bootstrap encoding
failure、firmware i64 length failure 仅 source review，未制造多 GiB request/
fake slice；现有 successful POST/ticket/status/size/batch/nonce tests 也通过。
HTTP peer/client futures bounded 2s join 允许 reset，非逐 peer EOF；owned 目录
Drop 清理。External 实际调用 public local IPSW/TSS，UID 在 network 前失败，
穷举三 variant。无真实 firmware/ticket/credentials/device/daemon/Apple/restore。

Before-final 29 PASS/15 FAIL 中 14 为 genuine；scan fixture 仅 gate Read，实际
到达 later member-read 而非 by_index_raw scan，排除该项错误期望的负控制证据。
初 before 同 fixture、尚未将多 case inputs 放在 source assertions 前。Public
before 为 0 PASS/1 genuine FAIL；首 after 为 43 PASS/1 fixture FAIL。Zip 2.4.2
open 缓存 data offset，scan 走 Seek；修为 Seek 也失败后，targeted 仅恢复旧 scan
adapter 的 before-corrected-scan 是 0 PASS/1 genuine FAIL，非完整旧 source rerun。
之后恢复 fixed adapter 再全部 final/full；失败日志保留，无 compile errors、
production guard/budget/lint 削弱，不把坏夹具或重复失败当新增 regressions。

Full 含 fmt/check/workspace strict Clippy/MSRV、66 features/四 groups、7+7
doctests/三 snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53 tools/118 help/38 raw、5 host workflows/22 device NOT_RUN、15 CLI/
3 backup contracts。Tested ios 0.1.13 debug SHA256
`675c638f8836ab8e4bfba1aa11beb4960f1498cbe731643b33e2fdea754b7ff4`。
源码 full 前后冻结；磁盘约 4.1/3.6 GiB，无 storage failure/cleanup；无额外
Chromium/installed 3.9/3.13。文档审查和收尾独立提交，不改 accepted 源码。

文档收尾：118 capability records、七个 changed Markdown 的 156 local links
（baseline a5bb2f4）、implementation/maturity/历史 evidence 保持、单个 host
entry 与 tested source commit 一致、whitespace PASS；docs-catalog/docs-links
日志同前缀。CHANGELOG/error/public/quality/catalog/roadmap/新专题与前专题
follow-up 共八个文件独立提交，accepted 源码/full 不变。

A02/第三类/总体 goal active，第四类未开始，全部 31 主任务/子任务保持。
下一确认合同缺口：legacy prefetch_tss_batch 将 combined/per-updater 原错误消费
为 failed_updaters 名称，无法区分 transport/codec 与 missing ticket。需提供
保留现有 constructible TssBatchResult fields、retry/nonce/wire 的 additive
observation API；read-only next-batch-observation/next-converters 日志同前缀。
其他 AFC/DTX/file/symbol/app/test/ZIP/GPX conversions、DProxy text、typed guards/
created outcomes/stream/JSON/callback/binding 与 L01/L03/A03/platform/device
条件继续；不升级 maturity、推送/发布、改写历史或修改 reference checkout。

### 大类 3：A02 TSS 批量回退失败观察交付（2026-10-03）

上轮 `d1a210f2ff466ae8f9e5f281e87ebb69823092fa` / 文档
`8d5dd2d0b6a139207765e4b05763d589c3e2f459` 后工作区 clean。本轮 PROGRESS，
非连续阻塞；分支 `feat/device-service-updates-20260927`。实现
`17f31a3efe91a77f6397c469cd3fd4ee3a65c0a4`，Linux x86_64，Rust 1.97.1 /
MSRV 1.80.1，workspace Python 3.12.13。专题
[TSS batch observations](validation/tss-batch-observations-2026-10-03.md)。

确认 legacy prefetch_tss_batch 在 combined build/sign 与 individual fallback
失败后只保留 failed_updaters，原错误及 recovered failures 无法观察。新增
prefetch_tss_batch_preserving_errors，与 legacy 共享算法，返回 private-fields
TssBatchOutcome / TssBatchFailure，以及从新增版本开始 non-exhaustive 的
phase/kind enums。三阶段区分 BuildCombinedRequest / SignCombinedRequest /
SignIndividualRequest；Error 保留原 RestoreError、typed class 与 native chain，
MissingTicket 区分成功响应缺失/空/非 data 票据，无虚构 error/class。Whole
combined failure index None，per-rider input index 保留 duplicate names 区别。
独立尝试不拼成伪造的一条 Error::source chain。

result/failures getters 借用，into_parts / into_error 保留独立 owned causes；
into_result 明确丢弃观察。Recovered error/missing ticket 仍可观察，即使最终
全部 ticket 成功；最终失败沿用 result.failed_updaters。Empty input 不调用
signer，invalid riders 在任何 signer 前 Err。Debug 只含 metadata/counts，
explicit original errors 遵守 source privacy，未导出为 CLI/HTTP/C/Python。

四个 source files（tss.rs/new batch.rs/new batch_contract_tests.rs/public
consumer），760 insertions/15 deletions。既有 rider/ticket/result constructible
fields、RestoreError 三 variants、legacy signature/root/features/deps/classifier
保持；merge/UUID/AP flags/validation/store/nonce/HTTP parsing/limits 与旧 tests
byte-identical。Baseline extraction 的旧算法只 outcome return wrapping 后
整份相同；最终 control-flow review 和五个分支情景检查两个入口 calls/order。
Legacy 不分配 observations storage，忽略的 signer errors 在下一 call 前释放；
new path 持有至 consume/drop，不对 caller-owned error payload 新设大小上限。
无新 timeout/retry/cancel/rollback/cleanup/reuse 合同。

Source diff SHA256
`d3d10bbe8dd576ea94121d35ce77bf69583a86716ace47b619ad46844dccbf05`，
四文件包含 staged new files。code-audit-final/baseline-audit/source-diff/original/
baseline 日志统一 `/tmp/ios-a02-tss-batch-` 前缀；一条临时 audit assertion 的
single-line formatting 假设以 whitespace normalization 修正，accepted source
未变化。源码 full 前冻结并保持至文档收尾。

| 验证 | 实际结果 |
| --- | --- |
| 新增合同 | 12 internal / 1 external |
| all-feature Restore | after-final.log 56 PASS |
| all-feature public consumer | public.log 32 PASS |
| minimum restore | minimal.log 56 PASS |
| minimum public consumer | public-minimal.log 16 PASS |
| Core strict Clippy | clippy.log exit 0 |
| 第 11 节 full | 首次 full.log terminal 0，2030 Rust / 13 Python，0 FAIL，2 既有 ignored |

实际 owned file-open OS NotFound/code、pending future 的 Tokio Elapsed、received
TSS UTF-8、local plist UID source 保留；real loopback reqwest timeout 在 fallback
成功后仍可读取。Fixture timer 不是新 public TSS issued deadline。沿用 no_proxy
50 ms HTTP helper，两 futures 2 s 内 join，peer tolerates reset，不声称逐 peer
EOF/设备 cleanup。缺失/空/非 data、merge conflict、duplicates、六 rider guards、
empty/full success、nonce exact/mismatch、private Debug、counted error drop 与
public unchanged construct/destructure/exhaustive family/owned native take 验证。

负控制使用新 outcome prototype 后的整份旧 extraction，观察未接入：
before-compiled.log 3 PASS / 8 FAIL，before-public.log 0 PASS / 1 FAIL，均缺失
failure observations；先行 result/call/validation checks 通过。before.log 与
before-final.log 两次 preliminary compile 因 key/type inference / trait-object
fn coercion 失败，修正后才运行 behavioral controls，不计 regressions。最后
新增 destruction test 未另跑旧 baseline；无 guard/budget/lint 削弱。

Full 含 fmt/check/workspace strict Clippy/MSRV、66 features/四 groups、7+7
doctests/三 snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53 tools/118 help/38 raw、5 host workflows/22 device NOT_RUN、15 CLI /
3 backup contracts。14 Rust result groups 与独立 Python host group 分别计数，
full-counts.log 可核对。Tested ios 0.1.13 debug SHA256
`471a5dec1fbf907f30e86684fcec1e388368b9a413cc6c72722a421b107d8a3f`。
Full launch / finish 约 3.0 / 2.4 GiB free，无 storage failure/cleanup；无额外
Chromium/installed 3.9/3.13。文档审查独立于 accepted full/source。

文档收尾更新 CHANGELOG/error/public/quality/catalog/roadmap/new topic/previous
restore follow-up 八文件；118 capability records、历史 evidence 和 maturity
保持，只增加受测 source 的 host entry。七个 changed Markdown 的 159 local
links 有效（baseline 8d5dd2d），whitespace PASS；docs-catalog/docs-links 日志
同前缀。
A02/第三类/总体 goal active，第四类未开始，全部 31 主任务/子任务保留。
下一确认 native input/source losses：AppService PID/plist、Testmanager PID、
ZIP integers 与 GPX parser/time，read-only next-input-sources.log。AFC/DTX/file/
symbols guarded converters、DProxy text、typed guards/created outcomes/stream/
JSON/callback/bindings 与 L01/L03/A03/platform/device 条件继续。未使用真实
device-generated requests/tickets/firmware/daemon/credentials/Apple operation；
不升级 maturity、推送/发布、改写历史或编辑 reference。

### 大类 3：A02 应用、测试、IPA 与 GPX 输入来源交付（2026-10-03）

上轮 source `17f31a3efe91a77f6397c469cd3fd4ee3a65c0a4` / 文档
`a2b69cc2777fc81b03b528ff4df375ca6f5e907e` 后工作区 clean；当前 PROGRESS，
非连续阻塞，分支 `feat/device-service-updates-20260927`。实现
`ab097900523e91f93e54249c42c214a7e1349ee6`，Linux x86_64，Rust 1.97.1 /
MSRV 1.80.1，workspace Python 3.12.13。专题
[service input sources](validation/service-input-native-sources-2026-10-03.md)。

保留 11 个此前 flatten/drop 的 native boundaries：AppService PID/plist（2），
Testmanager PID（1），ZipConduit entry/count/u32/u16/i64（5），GPX XML 与前后
RFC3339 timestamp（3）；合计 7 TryFromIntError / 1 plist / 1 DeError / 2 Parse。
AppService 使用已有 Xpc(Io)，其余使用已有 Io InvalidInput / private context。
Local PID/ZIP class 从 protocol 变 invalid_input；GPX 仍为 invalid_input，
GpxParse string 变 Io。Encoder private injected fault 的 nested IO kind/OS code
保留，wrapper 按 local serialization input。Returned variant/Display/immediate
payload/safe correlation 迁移明确；explicit source 遵守 redaction。

AppServiceError/DtxError/ZipConduitError/SimLocationError 既有 2/3/5/2 exhaustive
variants、public signatures/root/fields/features/deps/classifier/context/codecs
保持。无新 public API。固定 empty plist writer 在旧 adapter 基线前等价提取，
production 仍为 empty dictionary into Vec，42 成功 bytes 与旧实现一致。四份
extracted production sources 只 normalize native callbacks、private static phase
lifetimes 和拆开的 record count conversion 后相同；checked_add/其他 guards 与
原 ZIP decoder/read adapters 保持。Feature admission、control lookup/request/
ZIP init/metadata/CRC/entry/terminator/progress、GPX coordinate/optional paired
timestamp/rounding/negative clamp/wire/success 顺序不变。无新 budget/parser/
transfer cap/retry/cancel/rollback/cleanup/reuse 合同。

11 source files：5 production、4 new internal test modules、public consumer 与
旧 Testmanager integration，384 insertions/42 deletions。两条旧 AppService unit
assertions 与一条 integration assertion 按既定 variant migration 更新。最终
Source diff SHA256
`feffb2e4c54fe30f6822799cd5a66ab49ea0cdc65084df0eeb44d35c290df242`；
第一次 full 前 ten-file diff
`0370aa7472fb8e54873bc805e90c1b5b23486c0498ee0c33beaaed93921fb4df`
保留为 source-diff-first.patch。code-audit-final/original/baseline 等日志统一
`/tmp/ios-a02-service-input-`；encoder audit 的 formatter punctuation 假设以
callback/trailing-comma normalization 修正，未变 source。

| 验证 | 实际结果 |
| --- | --- |
| 新增合同 | 9 internal / 2 external |
| 旧 adapter 负控制 | before-final.log 0 PASS / 9 FAIL；before-public.log 0 PASS / 2 FAIL |
| 新 native contracts | after.log 9 PASS |
| Apps/Testmanager/GPX unit scope | apps/test/gpx.log 分别 57 / 16 / 8 PASS |
| 全 feature public consumer | public.log 34 PASS |
| minimum apps,testmanager | 7 native / 17 public PASS；AppService 需要 tunnel |
| minimum apps,testmanager,tunnel | 9 native / 18 public PASS |
| migrated Testmanager integration | all-feature 与 minimum 均 19 PASS |
| strict Core Clippy | clippy.log exit 0 |
| 第 11 节首次 full | terminal 1，Rust stage exit 101：旧 integration group 18 PASS / 1 FAIL |
| 第 11 节最终 full | full-final.log terminal 0，2041 Rust / 13 Python，0 FAIL，2 既有 ignored |

负控制 actual integer/XML/time faults；两个 PID builders 和两个 time positions
先 gather 再断言。Private writer 注入 captured owned File::open NotFound，保留
plist + nested IO / raw OS code，不当作 production Vec IO 或 filesystem write。
Empty success bytes 在旧 baseline source assertion 前通过。ZIP u32/i64 使用
private scalar/metadata helpers，u16 使用 65536-byte private header name；不分配
多 GiB assets。两个 usize→u64 failures 在本 64-bit host 不可发生，仅源码审查。
Owned public GPX replay / Testmanager absent-control PID / integration present
session+control 均检查 actual EOF / zero bytes，1 s bound，非 remote cleanup。
Private header zero-write 不是 whole install preflight：public ZIP input error
可能跟随已有 stream writes。Public GPX route fields/exhaustive families/native
getters 实际执行，App full XPC placement 源码审查，无物理服务调用。

第一次 full 揭示一条遗漏的旧 integration Protocol text 断言；更新为 Io/input/
TryFromIntError 与两 stream zero bytes 后，两 integration profiles 通过，再
完整重跑，production 未更改。保留 full-first-failed.log，不声称 first-pass。
无 compile failure/坏 fixture；一次 apply_patch formatting context mismatch
原子中止后重读再修正。未降 guard/budget/lint/验收标准。

磁盘 narrow 后 1.9 GiB，所有 known build handles terminal，确认无 live build
再仅 official cargo clean，报告 82403 files / 98.4 GiB，df 可用约 94 GiB。
不删源码/credentials/reference；tunnel minimum rebuild、两次 full 后最终约
69 GiB。clean-before/clean.log 保留，无 storage failure/再次 cleanup；每次 full
期间冻结 source，首 full terminal 后仅改已识别 integration。最终受测 ios
0.1.13 debug SHA256
`77a948d729afacab1a968d5de9dd34ad116eaa961e50539ac25f82e752aee43b`。
Full 含 fmt/check/workspace strict Clippy/MSRV、66 features/四 groups、7+7
doctests/三 snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53 tools/118 help/38 raw、5 host workflows/22 device NOT_RUN、15 CLI /
3 backup contracts；14 Rust groups / 独立 Python group，full-counts.log 可核对。
无额外 Chromium/installed 3.9/3.13。

CHANGELOG/error/public/quality/catalog/roadmap/new topic/previous TSS follow-up
八文档独立收尾，118 capability records / implementation / maturity / historical
evidence 保持，新增唯一受测 host entry，七个 changed Markdown 的 161 local
links（baseline a2b69cc）和 whitespace 审查通过，不改受测
source。A02/第三类/总体 goal active，第四类未开始，全部 31 主任务/子任务保持。
下一：guarded AFC/DTX/file/symbol sources 的 ownership/32-bit/unreachable 审查，
以及可达 DProxy native event/String 与 unchecked stderr 观察；read-only
next-guarded-sources/next-dproxy-observations 日志同前缀，已确认本地 zerocopy
0.8.42 map_src 可进行 owned source 映射，尚未实现该后续合同。其他 typed guards/
created outcomes/stream/JSON/callback/bindings 与 L01/L03/A03/platform/device
条件保留，无真实 app/test/install/location/device/daemon/Apple/credentials
动作、maturity 升级、推送/发布、历史改写或 reference edit。

### 大类 3：A02 DProxy 解码错误与部分事件观察交付（2026-10-03）

基线 `dfeb4500badf0932fe4f2df90ba1a14572f2e503`；受测 source
`a37a80b6eb7be4cc5f6d096e45eca1a65930e133`。Branch 仍为
`feat/device-service-updates-20260927`，Linux x86_64 / Rust 1.97.1 /
MSRV 1.80.1 / Python 3.12.13。具体合同和证据见
[DProxy decoder observations](validation/dproxy-decoder-observations-2026-10-03.md)。

新增 `StreamDecoder::push_preserving_errors` 和 opaque DecodeOutcome /
DecodeFailure。保留 events、独立原始 errors 和原 fatal DproxyError；同一次
push 先解码成功或 nonfatal error、随后 H2 fatal 时，前面的事件与来源仍可取出。
Direction/phase/kind/event_index/stream_id 均可程序化查询；新 enums
non-exhaustive。Metadata/count-only Debug 不输出 events/error payload。
into_parts 保留三条 owned branches，into_error 取原始对象；into_result 明确
丢弃 observations，并在 fatal 时丢弃之前 events，沿用旧 push 合同。
input_ignored 仅标记之前 DTX failure 导致本次跳过，普通 partial/Binary
无事件不等于 ignored。

六个原 error-discard adapters：plist、DTX、XPC length IO、XpcError 和两个
固定 TryFromSliceError。前四个有 runtime coverage，两个 fixed slices 在
>=24-byte guard 后不可失败，仅源码审查。u64::MAX body length 在本 64-bit
host 保留既有 InvalidData IO guard object，没有伪造 OS/TryFromInt cause。
Source-free decoder/Lockdown/H2/stream/message-size decisions 为 ResourceLimit；
END_STREAM incomplete 为 IncompleteMessage；既有 missing-stream/empty DATA
fatal 为 InvalidState，均无编造 native source/class。Fatal error class 仍由
原 DproxyError 分类，和 guard kind 分开。

四 source files，861 insertions/39 deletions。原四 variants exhaustive
DproxyError、ProxyEvent constructible fields/JSON、Direction/ProxyProtocol
和 root/features/deps/classifier 保持。Recorder/pump、stream limits/counting/
clear、H2 parser/preface、summaries/JSON/old tests byte-identical；整个三个
decoder bodies 与 admission 在观察/result-container normalization 后一致；
XPC parser 仅换 owned adapters，legacy error/overflow text 相同。DTX sticky
stop/ignore、XPC failed-stream clear 和 empty DATA fatal policy 保持，不将
后者升级为协议正确性认证。Legacy 不分配 observation storage。无新 timeout/
retry/reset/rollback/cancellation/forwarding/cleanup/reuse 保证。
Source diff SHA256
`9fe368edc4102b30acb4ea579462fb9e9ab173c59034b1571a33e18c207eb730`。

| 验证 | 结果（日志 `/tmp/ios-a02-dproxy-observations-`） |
| --- | --- |
| 原算法负控制 | before-final.log 2 PASS / 11 FAIL；before-public-final.log 0 PASS / 1 FAIL |
| all-feature / minimum dproxy | after/minimal.log 均 25 PASS |
| public all-feature / minimum | public/public-minimal.log 35 / 16 PASS |
| strict Core Clippy | clippy.log exit 0 |
| 第 11 节首次 full | full.log terminal 0，2055 Rust / 13 Python，0 FAIL，2 既有 ignored |

新增 13 internal / 1 external tests：真实 malformed binary plist、DTX
bad magic、nested XPC UTF-8 source、existing SDK length IO、owned errors、
native/success event 后 fatal、source-free guards、实际 33 tiny stream flood、
64 MiB+24+1 admission、fragmented all-protocol result/state、public exhaustive
consumer。Legacy/observed comparison 共享 final engine；另有 exact old
algorithm prototype 和 whole-body audit 支持旧行为。Timestamps 为 wall-clock，
不列入值相等断言。第一轮 plain-text plist fixture 被 parser 接受，其 raw
fallback assertion 无效；改为 binary malformed 并先独立确认 is_err 后，
两 controls 完整重跑，坏 fixture 不计 regression evidence。无 compile failure
充作证据；audit match-arm/trailing-comma normalization 修正未改源码。

Full 含 fmt/check/workspace strict Clippy/MSRV、66 feature builds/四 groups、
7+7 doctests/三 snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53 tools/118 help/38 raw、5 host workflows/22 device NOT_RUN、15 CLI /
3 backup contracts；14 Rust groups / 独立 Python group，full-counts.log 可核对。
无额外 Chromium/installed 3.9/3.13。受测 ios 0.1.13 debug SHA256
`64ae2015597f0d01173a41bc135837faadebac53616e0eaa264000860e8ea096`。
所有 source 包括 new files 在 full 前 staged，full/docs 期间冻结；本 slice
controls 后约 67 GiB free、收尾约 60 GiB，无 cleanup/storage failure。

八文档独立收尾；118 capabilities 的 implementation/maturity/history 保持，
新增唯一 host evidence；七个 changed Markdown 的 166 local links、whitespace
及 accepted source/binary hashes 审查通过（doc-audit.log）。
A02/第三类/总体 goal active，第四类未开始，全部 31 主任务/子任务保持。
下一：Recorder/CLI error observers 和三个 unchecked stderr sites；它们仍用
legacy push，本次新 API 不代表已完成调用方集成。Read-only next-recording.log
记录实际位置。随后 guarded AFC/DTX/file/symbol ownership/32-bit 转换；其他
typed guards/created outcomes/stream/JSON/callback/binding 与 L01/L03/A03/
platform/device 条件保留。无 device/proxy/daemon/credentials/Apple 实操、
reference edit、push/publication、历史改写或 maturity upgrade。

### 大类 3：A02 DProxy 录制观察与 stderr 错误交付（2026-10-03）

基线 `17abc82054b8a5480e9cf34b4e0bed949891339a`；受测 source
`4dda7a225a10f15e22d0bd5de47c7a5b6624ca83`。Branch 仍为
`feat/device-service-updates-20260927`，Linux x86_64 / Rust 1.97.1 /
MSRV 1.80.1 / Python 3.12.13。合同与证据见
[DProxy recorder/CLI observations](validation/dproxy-recording-observations-2026-10-03.md)。

新增 record_chunk_with_observer / proxy_bidirectional_with_observer，FnMut
callback 逐项取得 owned DecodeFailure；async proxy 的 callback 要求 Send，
两 direction 在既有 recording mutex 内串行，当前 chunk 转发之前调用。
Raw 写先于 decode，raw failure 不 decode/callback，仍可能留下 partial raw。
Callbacks 在 events 写入／原 fatal 返回之前交付，后续 Serde/newline/diagnostic/
flush error 不抹去已交付的原始来源。Primary Result 与 decode observations
为独立 owner，非虚构 source chain。Fatal decode 仍跳过该 push 的全部 events；
failure event_index 为 decoded push 内索引，非 persisted line/global index。
需要 owned partial events 的调用者仍直接用 StreamDecoder outcome。

SDK 不累计 session failure list，legacy record_chunk/proxy_bidirectional 保持
unobserved mode，无 observation allocation。Caller 自行限制 retention，并
避免 blocking；callback panics 不捕获。无新 timeout/retry/reset/rollback/
cancellation/durability/remote cleanup/reuse 保证，ordinary decode errors
仍为 nonfatal。CLI 改用 observer，DEBUG（-vv）仅写 metadata/count-safe
Debug，不写 native Display/source/raw payload，既有 tracing facility 保持。
Core event 和两 CLI status stderr sites 由 panic 改为原始 IO Err；既有
dispatcher operation failure exit 1／best-effort final reporting 保持。Status
调用位置在 initial connect/bind／capture creation 之后，仅源码审查，未运行
物理 DProxy command。Raw/NDJSON effects 可早于诊断 failure，非 rollback。

四 source files，710 insertions/23 deletions。原四 exhaustive DproxyError
variants、ProxyEvent fields/JSON、root/features/deps/classifier/observation
types 和 DTX/H2/XPC codecs 保持。整个 decoder/parser/old tests byte-identical，
仅当前 adoption doc sentence 更新；recorder construction/meta/file layout/
redaction 不变。共享 proxy 保留 splits/try_join/mutex/16 KiB read/record-before-
forward/flush/shutdown；whole production CLI 仅改两个 checked status writes
和 observer adoption。Private event writer 等价提取 JSON/newline/diagnostic/
flush，原 panic policy 单独保留在负控制 prototype，final 在同一步返回 IO。
Source diff SHA256
`a72d1ec7cec1ecaf164bbed70c704f2ffc25538a992db34d231798bcefb29771`。

| 验证 | 结果（日志 `/tmp/ios-a02-dproxy-recording-`） |
| --- | --- |
| 原算法 callback controls | before.log 初始 10 tests：3 PASS / 7 FAIL |
| public / CLI metadata controls | before-public.log / before-cli.log 各 0 PASS / 1 FAIL |
| all-feature / minimum dproxy | after-final.log / minimal.log 均 36 PASS |
| CLI DProxy scope | cli.log 10 PASS |
| public all-feature / minimum | public.log / public-minimal.log 36 / 17 PASS |
| strict Core/CLI Clippy | clippy.log exit 0 |
| 第 11 节首次 full | full.log terminal 0，2068 Rust / 13 Python，0 FAIL，2 既有 ignored |

新增 11 internal / 1 CLI / 1 external tests。Malformed plist/DTX/XPC native
objects 在 callbacks 保留；实际 read-only raw/events File 产生 OS write
errors，验证 raw-before-decode／source-before-events-failure。Private writer
注入 captured actual File::open NotFound，覆盖 JSON/newline/diagnostic/flush，
native kind/OS/class 保留，不当作实际 sink syscall。Diagnostic failure 不
panic，已有一完整 NDJSON line、尚未 flush／第二 event。额外 Linux child
--nocapture 将自身 stderr 定向 /dev/full，public record_meta_event 返回真实
ENOSPC/OS28/IO，child success stdout marker、已有一 NDJSON line；不影响
parent stderr，不代表 CLI physical startup。该 test 在 controls 后新增，未
声称运行旧算法负控制；cfg Linux，不推广其他平台 syscall acceptance。

两个 joined duplex fixtures 有 1 s bound，实际双向 bytes/raw/EOF 与 fatal
zero-forwarding/empty event file，证明 local owned stream destruction，非
remote acknowledgement。Broken DTX 后仍留 raw，不重复 decode source；
empty input 跳过 read-only writers。CLI actual subscriber capture 验证 safe
metadata/class 且排除私有 payload/hex；public recorder 实际调用，public async
signature 编译后 future 未 poll，Core tests 另行实际运行两方向。Controls
失败为 missing observations/metadata 或 IO panic，非 compile/invalid fixture。
code-audit-final.log 核对精确 old bodies 与 whole-production 变更，无降 guard/
lint。所有 source（包括新文件）在 full 前 staged，full/docs 期间冻结。

Full 含 fmt/check/workspace strict Clippy/MSRV、66 feature builds/四 groups、
7+7 doctests/三 snippets、C ABI/malformed/checked list、3.12 wheel/sdist rebuild/
install、53 tools/118 help/38 raw、5 host workflows/22 device NOT_RUN、15 CLI /
3 backup contracts；14 Rust groups／独立 Python group，full-counts.log 可核对。
无额外 Chromium/installed 3.9/3.13。受测 ios 0.1.13 debug SHA256
`2f020e35dc0879d17d67afbc75844bd9779808a4beecb1939fd9353af08d248c`。
Narrow 后约 59 GiB free，收尾 58 GiB；无 cleanup/storage failure，full terminal。

八文档独立收尾，118 capability records 的 implementation/maturity/history
保持，新增一个受测 host entry；七个 changed Markdown 的 171 local links、
whitespace 和 accepted source/binary hashes 审查通过（doc-audit.log）。
A02/第三类/总体 goal active，第四类未开始，全部 31 主任务及子任务
保持。下一 guarded AFC/DTX/file/symbol ownership／32-bit sources，再继续
其他 typed guards/created outcomes/stream/JSON/callback/binding；L01/L03/A03/
platform/device 条件保持。无 physical proxy/device/daemon/credentials/Apple
action、reference edit、push/publication、历史改写或 maturity upgrade。

### 大类 3：A02 guarded AFC/DTX/file/symbol 原始来源交付（2026-10-03）

实现与受测提交：`37b09e0bd822bce725e74a01a78eedaeceb4688e`，基线
`e6e274ea175e327f00b8abb337bda69c7491b430`；分支保持
`feat/device-service-updates-20260927`。
[专题证据](validation/guarded-native-sources-2026-10-03.md)记录原生所有权与可达性。

八个原生转换 adapter（AFC5、DTX1、FileService1、symbols1）从文本 Protocol
转为既有 Io/private received context：InvalidData、固定阶段、optional full-width
declared u64，分类保持 Protocol。AFC 使用 zerocopy map_src 保存原 CastError
kind/bytes 的 Box<[u8]>；仅失败复制调用方固定40字节，成功 borrowed decode保持。
Debug 不打印 native payload，显式来源仍受隐私合同约束。

六处在当前守卫下任何平台均不可达：header sizeof40/align1、四处固定长度
slice，以及 AFC this_len（先 this<=entire，且 entire 已成功转 usize）。
只有 AFC entire_len 与符号 count 可能在 narrower usize 失败。本64位环境
八处 production native failure 均未运行；u64::MAX 测试走既有 frame/count ceiling。
私有39字节 CastError fixture 验证所有权，真实 u32 overflow/七字节 slice 验证
generic received adapter，不冒充 production branch coverage。32位条件测试分支已
编译但未执行，runtime证据继续 NOT_RUN。

七个源文件231 insertions/27 deletions，新增四内部/一external测试。整个生产
body只归一化八个callback并去掉新增private helper后与baseline一致；guards/
read/order/allocations/status/magic/limits/wire/success、原测试与public families/
fields/signatures/root/core feature list/classifier/layouts/H2/XPC保持。对应错误的
returned variant/Display/immediate-source/diagnostic可能变化；按 typed class/source
迁移。既有 zerocopy 0.8 std feature 为MSRV原生StdError所需；暂时关闭后最低
afc check真实E0277/exit101，恢复后同check exit0。该失败是依赖编译条件，不是
behavior negative control；无版本或lockfile变化。

日志 `/tmp/ios-a02-guarded-sources-`：

| 验证 | 实际结果 |
| --- | --- |
| 新内部 all/min combination | 4/4 PASS |
| AFC/DTX/fileservice/symbol | 20/46/18/11 PASS |
| Public all/min combination | 37/17 PASS；owned joined duplex真实request/EOF，1秒有界 |
| restored最低afc MSRV / strict core Clippy | terminal exit0 / exit0 |
| 首次canonical full | terminal exit0；2073 Rust/13 Python PASS、0FAIL、2既有ignored，14 Rust groups |

Full是第11节统一入口，Python3.12.13：fmt/check/strict workspace Clippy/MSRV、
66feature/四组、7+7doctest/三snippets、C ABI/malformed/checked list、3.12wheel/
sdist、53tooling/118help/38raw/五host/22device NOT_RUN/15CLI/三backup合同均PASS。
未单独重跑Chromium或安装3.9/3.13。source全程冻结，无live handles；约55GiB
narrow后、34 GiB结束，未cleanup/未storage failure。
source diff SHA256 `859736faeb608d2c6aa836d05ba58515c50488348d8f30a1194e1f16d5e1a277`；
ios0.1.13 debug SHA256 `a3820d7a3c9009b4bfe32ea910316c81ec8a54b2bdaf5519041097fd6183fbba`。
八文档另行收尾，七MD/175 local links与hash/whitespace通过；118capabilities/
implementation/maturity/history保持，仅增加一条明确host entry。

A02/第三类/总体goal active，第四类未开始，全部31主任务与所有子任务保持。
下一为mDNS/CDTunnel及剩余connection native causes，再继续typed decisions/
stream/JSON/callback/binding；L01/L03/A03/platform/device条件保留。未操作物理
设备/daemon/credentials/Apple、未修改reference、未push/publish/rewrite history或
升级成熟度。

### 大类 3：A02 discovery/connection 原始来源交付（2026-10-04）

受测实现：`4b3a9cbe40b696b01cbfdb65ba8a7d614795e9fc`，基线
`a2c726785a0fcb3601468619dfb74369366a0139`，分支保持。
[专题证据](validation/connection-native-sources-2026-10-04.md)区分实际wire/timer、
构造backend对象与source-reviewed TCP expiry；检查启动于10-03、验收于10-04。

15原始对象保留：mDNS create/browse4、MTU1、handshake7（包括原 .ok 丢失的
port overflow）、userspace port1、RSD check-in1、lockdown dial1。既有Io/private
fixed context，mDNS opaque仍Unknown；本地MTU为InvalidInput（minimum guard无
native source，overflow保留TryInt/full u32）；received port/MTU/JSON仍Protocol，
包括收到的MTU套住原始本地validation error。真实CDTunnel Elapsed从文本Protocol
转Timeout，RSD/dial仍Timeout，保留configured Duration/native Elapsed。Debug不含
任意native/address，dial文本不再包含raw host/port。variant/Display/source/class
迁移记录；3 TunnelError variants/root/fields/signatures/binding schemas保持。

七源文件393 insertions/106 deletions；whole production仅callback/minimum guard
error representation/private helper归一化后与baseline一致，port展开单独核对u16/
missing/zero/success等价。wire/requests/magic/ranges/limits/allocation/shared budget/
read/write/task/daemon/cleanup保持，classifier/deps/features/public consumer/layout/
AFC+DTX/H2+XPC保持。无新SDK export、deadline/retry/invalidation/reuse保证。
Fixed numeric JSON into Vec及<=255后的u16 length failure不可达，未伪造覆盖。
TCP dial expiry只审查原15s，没有black-hole/device运行；mdns对象fixture不能证明
真实multicast daemon或已被mdns-sd擦除的深层OS cause。

五新增内部tests：joined owned duplex实际坏JSON/port/MTU/native chains、exact
request/EOF、真实zero-budget CDTunnel/旧20ms RSD Elapsed；owned loopback preamble
u32 overflow/native/empty reply/EOF。1秒joins，不声明remote清理ack。旧握手先
9PASS/6FAIL（variant断言），迁移六项后15PASS；候选private-path external consumer
两E0603后移除，原public文件byte-identical，没有扩大API；这不是behavior control。

| 验证 | 实际结果 |
| --- | --- |
| 新内部 all/minimum mdns,tunnel-userspace | 5/5 PASS |
| handshake/discovery/routing/lockdown/userspace | 15/9/11/4/16 PASS |
| existing public all/minimum | 37/17 PASS |
| strict core Clippy | terminal exit0 |
| first canonical full | 最终PASS marker；2078 Rust/13 Python PASS、0FAIL、2既有ignored |

日志 `/tmp/ios-a02-connection-native-`。Full第11节fmt/check/strict workspace
Clippy/MSRV、66feature/四组、7+7doctest/三snippets、C ABI/malformed/checked list、
3.12wheel/sdist、53tooling/118help/38raw/五host/22device NOT_RUN/15CLI/三backup
合同PASS。无额外Chromium/installed3.9/3.13。续接后原handle missing，最终PASS
marker及workspace无live build已核对；原shell exit未跨续接捕获，脚本仅全scope
成功才输出该marker后normal return；未重启同一full。所有source staged后冻结。
Source diff SHA256 `d7fa5e8225c0eeccaeae4fe531eb535aec3d70db82d3c6b80210ce049f0f30aa`；
ios0.1.13 debug SHA256 `2efdd42e0b320ce98f0356c3f44ba3a8761ecee0ad406e567ddec76639b078c3`。
Full前33GiB/后20GiB，未cleanup/storage failure。八文档收尾，七MD/179 local
links与hash/whitespace通过；118capabilities/implementation/maturity/history保持，
仅新增host entry并更新catalog日期。

A02/第三类/总体goal active，第四类未开始，31任务与所有子项保持。下一remaining
filesystem native adapters/checked CLI diagnostics，并有限审查typed decisions/
stream/JSON/binding范围；L01/L03/A03/platform/device保留。无实际设备/daemon/
credentials/Apple action、reference edits、push/publication、history rewrite、
maturity upgrade。

### 大类 3：A02 CLI diagnostics、CPU partial 与目录 NUL 来源交付（2026-10-04）

从 clean `d12449c810fe6564e0f3cfcd32ac0f15cef223db` 继续，分支不变；本轮属于
已验证进展。源码 `5f259b8f90dd40f973085a00b401a37a62066ac1` 已聚焦本地提交，
17 文件、467 insertions/103 deletions。Linux x86_64、Rust 1.97.1/MSRV 1.80.1、
Python 3.12.13；[专题证据](validation/cli-diagnostic-sources-2026-10-04.md)。

60 个直接 eprintln 改为既有 checked stderr helper，54 处传播原生 write/flush
IO，6 处保留已有主失败（credential partial、accessibility cleanup、CDP 两处、
screenshot callback、CPU read）。14 份完整 production body 与 reviewed replacement
比较一致；60 组原诊断参数保持，CPU callback 仅 local borrow 名变化。CPU 私有
renderer/loop 等价提取单独审查；共享 writer 只增加 crate visibility，body 不变。
目录两个 CString adapter 保留 original owned NulError、Input 与 fixed safe context，
原 bytes/position 可显式检查；syscalls/path policy 不变，拒绝在 rename 前发生。

CPU read 原 break/stop/Ok 改为同一 stop attempt 后返回 original DtxError，已有
JSON row 保留，diagnostic secondary 不覆盖主失败。finite JSON/human format 保持；
NaN/±infinity JSON 在该行输出前 source-free Protocol 拒绝。Count/deadline/Ok(None)
正常完成、ignored stop error 和 stdout failure 原返回位置保持。不承诺 remote
cleanup ACK/reuse/retry/rollback。Stderr failure 从 panic 转正常 exit1，partial CPU
read 从 false success 转 exit1；schema/roles 不变，普通状态可能已在 effects 后。

| 检查 | 实际结果 |
| --- | --- |
| 新增测试 | 3 CLI/1 core；actual ENOSPC child、owned DTX partial EOF/stop、nonfinite 与 NUL 来源 |
| 原实现/旧行为对照 | 原 eprintln 已编译后 0 PASS/1 FAIL；CPU extracted prototype 0 PASS/2 FAIL，后逐字恢复 |
| 相关检查 | CLI 667 PASS/1 既有 ignored；filesystem 4/minimum 新测试 1 PASS；fmt/strict core+CLI Clippy terminal0 |
| 第 11 节 full | 首次 full.log terminal0，2082 Rust/13 Python、0 FAIL、2 既有 ignored |
| 实际 binary 子进程 | pair/pair wifi × human/JSON，stderr /dev/full：4 PASS，exit1/stdout0，在发现前失败 |

CPU fixture preparation 的 E0283、错误 hardcoded identifiers 与非递归 archive 已修正，
不算行为 negative control。真实 peer EOF/native DtxError、原 row、stop frame 与
注入 reporting PermissionDenied 的 primary 保留在一秒 joined duplex 内检查；并非
设备/remote cleanup。Actual binary child 3 秒限额，不因 missing stderr envelope 宣称
成功。Cargo/root/classifier/public consumer/DTX/C/Python 与 binding schema 保持。

Full 同前统一命令，14 Rust/1 Python groups、66 feature/four groups、7+7 doctests/
three snippets、C ABI/malformed/checked list、3.12 wheel/sdist、53 tooling/118 help/
38 raw/five host/22 device NOT_RUN/15 CLI/three backup contracts 全 PASS。
未重复 Chromium 或 installed 3.9/3.13。日志 `/tmp/ios-a02-cli-diagnostics-` 前缀。
Source diff SHA256 `423b46a2737982713d06d837e25829a1d9530cdbc10ae2e6bb911e85411c6e85`；
ios debug SHA256 `1f45befb8217367d540337f251b7b390feac03074ccbf2967f0114cbad926d4b`。
Narrow handles 全停后 official cargo clean -p ios-cli 清理 6336 files/16.8 GiB，
无手删 target；full 前约30GiB/后27GiB，未 storage failure/restart。Source staged
并冻结，全验收后提交；八文档收尾保持 118 capabilities/maturity 与完整既有 evidence。
七 Markdown/178 local links、whitespace 与 exact source/binary hash 检查 PASS，
catalog 只增加一项 host entry，implementation/maturity/history 保持。

A02/第三类/总体 goal active，第四类未开始，完整31任务/子任务保持。下一批处理
剩余 CLI native/Elapsed adapters，再有限审查 source-free decisions/stream/binding
验收；110 处 map_err/anyhow 盘点含 unit/取消决策，须依据真实类型处理，不按自然
语言分类。L01/L03/A03/platform/device 条件保持，无实际设备/daemon/credentials/
Apple action、reference edits、push/publication、history rewrite 或 maturity upgrade。

### 大类 3：A02 CLI 原生来源与 typed cancellation 交付（2026-10-04）

从 clean `1f0b67fba777e53fcee2fe44fb3b5b2e869ee2be` 继续，本轮为已验进展。
源码 `03c71b53cfd653636733662cbf3a241a4c7ed10f` 已本地提交；同一分支，27 CLI
文件、567 insertions/137 deletions。Linux x86_64、Rust 1.97.1/MSRV 1.80.1、
Python 3.12.13；[专题证据](validation/cli-native-sources-2026-10-04.md)。

105 original objects：81 service/plan/clock，13 actual Elapsed，六 local input
（coordinate2/IPv6/profile/bool/float Duration），三 received（focus hex/DDI2），
两 file IO（GPX/P12）。Native service 保留 original category/Unknown，timer 为
Timeout；local/received wrappers 为 Input/Protocol，fixed private Debug 保留 native
source，kind 是 provenance 而非 OS fault。四 legacy parser String 保留 owned opaque
data/sourceNone，四 URL unit adapters 与五 direct decisions 不制造 native cause。
三个取消入口使用 existing OperationCancelled；HID signal 注册 IO failure 不再当
取消。预算、select order、request/wire/limits/success/cleanup 不变，没有新的
remote ACK/reuse/retry/rollback/no-effects 保证。Display/source/category/correlation
迁移；GPX/P12/coordinate fixed contexts 去 raw input/path，schemas/exit policy 保持。

| 检查 | 实际结果 |
| --- | --- |
| 新增测试 | 11 PASS；original five 已编译后 0 PASS/5 FAIL，随后恢复 PASS |
| 准备/审查修正 | typed-plan fixture 缺 --xctest 导致 9 PASS/1 FAIL；Clippy needless_return 已修；focus 来自 received event，full 前修正 Protocol provenance |
| 相关检查 | 早期 CLI 677 PASS/1 既有 ignored；最终 new11/fmt/strict CLI Clippy terminal0 |
| 第 11 节 full | 首次 full.log terminal0，2093 Rust/13 Python、0 FAIL、2 既有 ignored；最终 CLI678 PASS/1 ignored |
| Actual CLI machine fixtures | six PASS：lat/lon/GPX missing/native Duration overflow/nonfinite guard/native plan，exit1/class/phase 正确，stdout/raw0，trap connection0，owned child reaped |

Actual pending timers、native parser objects、host profile builder/plan error 与 actual
empty-path OS file failure 受测；signal future 注入 captured file-open OS error，
不是 signal-registration syscall fault。Opaque data/native contexts 的安全 Debug 与
source-free guards sourceNone 有证据。Old location control 首个 Input assertion 已
失败，后面的 file subcase 未在该 negative control 到达。无设备或 DDI/TSS 验收。

Code audit 113 callback changes 逐项对照（native105/opaque4/unit4），27 complete
production token bodies 在 reviewed adapters/五 guards/三取消/two private extractions/
unused import/one formatter comma 以外一致。Audit script 修正 continued strings/
char literals/closure block/comma 的解析，没有改 frozen product source。Core/root/
classifier/public consumer/deps/C/Python byte-identical，所有源码 staged 后冻结。

Full 同前统一命令，14 Rust/1 Python groups、66 feature/four groups、7+7 doctests/
three snippets、MSRV、C ABI/malformed/checked list、3.12 wheel/sdist、53 tooling/
118 help/38 raw/five host/22 device NOT_RUN/15 CLI/three backup contracts 全 PASS。
未重复 Chromium 或 installed 3.9/3.13。日志 `/tmp/ios-a02-cli-native-` 前缀。
Source SHA256 `8fe550bf0579e62bd038ca0438f23a2b44d229fdfe710fdb431a926ad3c0a4cb`；
ios debug SHA256 `863bce33663dde0d69643f1bc08bf4090a03caee72e0f8f4f8e7582f3212e2c0`。
Full前约24GiB/后21GiB，未cleanup/storage failure/restart。八文档收尾保持118
capabilities/implementation/maturity 与完整历史，只新增一项 host evidence。
七 Markdown/181 local links、whitespace 与 exact source/binary 检查 PASS。

A02/第三类/总体 goal active，第四类未开始，31任务及所有子项保持。下一步有限
核对 source-free decisions、stream/JSON 和 binding projection 的真实合同验收，
再按依赖进入 lifecycle 类；L01/L03/A03/platform/device 条件保留。没有实际设备/
daemon/credentials/Apple actions、reference edits、push/publication、history rewrite
或 maturity upgrade。

### 大类 3：A02 CLI 流错误与 framing 合同交付（2026-10-04）

实现提交 `64153aae931a2b2b147ef4611ba1d5f36f9ea661`，前置文档基线
`bc8614ba29aaba30d1fe22635a4885c83e30b40d`。Linux x86_64，Rust 1.97.1/
MSRV 1.80.1、Python 3.12.13；九个 CLI 文件 340 insertions/53 deletions。
专题：[流错误、framing 与主机证据](validation/cli-stream-contract-2026-10-04.md)。

Pcap 原 native read error 和 FPS reader→channel→CLI 的 DtxError 不再转 String；
两处 XCTest wait、notification stream 与 CrashReport version 的 actual Elapsed
保留。OS Trace 两个 signal wait、crash watch、power assertion hold 区分 actual
signal IO failure 与正常 operator stop。共十条原 native source-loss paths；
crash/power 信号分支仅 source-reviewed，不制造生产 syscall fault 证据。

Notification observe/stream 的 ProxyDeath event 使用现有 typed closed-proxy error，
class disconnected，无虚构 IO cause。Companion/OS Trace 的 issued deadline 是
source-free IO TimedOut，无 OS code/Elapsed。既有 operator stop success、count/
capture window 和 native failure exit 保留。JSON WebInspector shell 去掉交互 prompt
与 EOF presentation newline；human/成功 JSON records 保留。此两 guard 的行为为
source-reviewed，无 PTY/真机交互验收。返回 class/Display/source/correlation 可变，
machine v1/exit 1/unknown effects 保持。先前有效 records 或文件/设备 effects 可存在。

新增八测试：actual owned duplex Pcap expiry/EOF、notification escaping/framed
ProxyDeath/Elapsed/EOF/safe machine envelope，actual DTX EOF 经过 owned FPS channel，
actual timers→private XCTest/Crash adapter，issued source-free markers，OS Trace
两 deadline modes 注入 actual empty-path file-open IO 作为 owned signal future。
不是完整 CoreProfile setup/reader/stop，不向 parent 发 signal，也不是真实 signal
registration 失败。首个 fixture 错用 private Core module 出 E0603，改为 fixture 自有
plist framing 后通过；没有 old-algorithm negative control 或 test-only public export。

两最后 signal/prompt guards 前 CLI 全量 686 PASS/one old ignored；final fmt/strict CLI
与 first canonical full terminal0。Full 2101 Rust/13 Python、CLI686 PASS/one old ignored、
zero FAIL/two old ignored；14 Rust/1 Python groups、66 feature/four groups、7+7 doctests/
three snippets、MSRV/check/workspace strict Clippy、C ABI/malformed/checked listing、
3.12 wheel/sdist、53 tooling/118 help/38 raw/five host/15 CLI/three backup contracts PASS，
22 device rows NOT_RUN。未重复 Chromium 或 installed 3.9/3.13。
命令 `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`，
session27804 terminal0；日志 `/tmp/ios-a02-stream-native-` 前缀。
Source SHA256 `2a95e4665c28c5215b0e1147d08845810e280410af99e5566b90c151188fb3c8`；
ios debug SHA256 `837a8d2d7b609a745c2821665cc2f39fb989c0b64e01003378df8a59b817197f`。
九 source files staged/frozen through full；Core/C/Python/deps byte exact。Production
diff reviewed；hash 是 freeze evidence，不是所有 device operations 的行为证明。
Full前后约21GiB，未cleanup/storage failure/restart。

专题登记 NDJSON、successive pretty documents、aggregate summaries、per-record partial
failure 和 existing stop policy；error-format=json 只约定 final stderr error，不将
progress/tracing stderr 整体声明 JSON。绑定/probe 的现有 scoped mapped profiles 复核：
C ALLOC 的 NUL/alloc ambiguity 保持 unknown，Python native metadata/typed builtin args
与 RSD initial/skipped/metadata/service-missing 的 evidence 不推断 device unsupported。
FPS unbounded channel/task ownership、stop/join/flush placement 与更强 reuse/cleanup
继续属于 L01，不把分类结果解释为 remote ACK/retry/rollback。

八文档收尾，七 Markdown/187 local links、whitespace、exact source/binary/full log
与 catalog PASS；118 capability/implementation/maturity 和完整历史保持，仅增一项
host evidence。文档不替代 device maturity/真机验收。

A02/第三类/总体 goal active，第四类未开始，31任务及所有子项保持。已定位的下一步：
backup 的 MissingValue 文本决策、debugserver 的 InvalidService 文本决策，以及
owner-issued source-free guards（含 accessibility empty traversal timeout）的有限复核。
平台/真机/部署/fixture 与 L01/L03/A03 条件保留；本轮不更改 capability maturity，
不操作真实 device/daemon/credentials/Apple、reference、push/publication 或 history。

### 大类 3：A02 Lockdown 响应码与 CLI 决策交付（2026-10-04）

实现提交 `08df6533265656b394b58a829653b15dfa7d197f`，前置文档基线
`b2f81b2a07dfac6e9c4a7ed9f2a3218a26c96eee`。十个源码文件，853 insertions/
33 deletions。专题：[Lockdown 响应码与主机验收](validation/lockdown-response-codes-2026-10-04.md)。

新增 opaque `LockdownResponseError`，保留 owned operation/code，公开 typed getter
及最多32层的 source 查询；Debug/Display 隐去原始码。GetValue/MissingValue 归类
NotFound，StartService/InvalidService 归类 ServiceMissing，其余明确失败为
ServiceFailure，不推断 DDI/设备支持/重试。新的 preserving Rust GetValue/StartService/
ConnectedDevice API 共享既有传输；旧 API 保留 Protocol(String)、exhaustive families
和成功结果。新 GetValue profile 拒绝 non-string Error，旧方法行为保持。

Backup 三个 WillEncrypt 查询仅对 observed exact MissingValue 默认 false，其他原始
error 继续传播。Debugserver 保留候选顺序，仅 exact InvalidService 回退，两次缺失
返回最后 owned response error。文本 substring 决策移除；machine v1/exit1 保持，
class/Display/source/correlation 可迁移。不是所有 API 都已产生 response observation。

新增七测试（Core 三内部/一公开，CLI 三），迁移一旧文本断言。实际 owned duplex
检验公开旧/新请求和错误优先级；GetValue 比 exact bytes，StartService 比 parsed
objects。实际内存 rustls QueryType/StartSession/GetValue 经过 backup resolver，
debugserver 实际 private candidate loop 验证两顺序/一或两次请求/最终错误/安全渲染。
测试还覆盖边界32/cycle/panic-on-format、非字符串 Error、native EOF/host NotFound。
TLS server no-client-auth；不是 Apple pairing/trust、ConnectedDevice/usbmux 或真机
流程验收。准备 E0283/E0505 fixture 错误已修，未运行旧实现 negative control。

Linux x86_64，Rust1.97.1/MSRV1.80.1、Python3.12.13。Canonical full 命令
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`，
session83948 terminal0、final PASS；2108 Rust/13 Python，CLI689/Core1271/公开38
PASS，zero FAIL/two old ignored。fmt/check/strict workspace Clippy/MSRV、66 features/
four groups、7+7 doctests/three snippets、C ABI、3.12 wheel/sdist、53 tooling/118 help/
38 raw/five host/15 CLI/three backup contracts PASS，22 device rows NOT_RUN。
未重复 Chromium 或 installed3.9/3.13。Source SHA256
`55b7b8dade1e7de91cc9dd4fc62cd90bbaa0e5ce57b5752ee75ceeba0e3f37f0`；
ios debug SHA256 `2e0650178e80fb30b7f5fc6feb45388e0ea8713233d1c84319ed1fb0bd2c7616`。
十源文件保持冻结至 full terminal，约20GiB余量，无 clean/存储失败/restart。
日志 `/tmp/ios-a02-lockdown-codes-`；两个 reference checkout 状态及 commit 保持，
只比较协议行为。C/Python/dependencies 和原有 error enum definitions 未改。

八文档收尾，七 Markdown/191 local links、whitespace、exact hashes/full log 和 catalog
检查 PASS；118 capability/implementation/maturity 和所有历史 evidence 保持，只增
一项 host evidence；记录不替代真机验收。A02/第三类/总体 goal active，第四类未开始，
完整31任务及所有子项保持。下一有限复核已定位13个 owner-issued timeout/input/
precondition guards（含 accessibility empty traversal timeout）；不是全部 legacy error
清单。L01/L03/A03/platform/device 条件保留，无真实设备/Apple actions、reference
修改、push/publication、history rewrite 或 maturity upgrade。

### 大类 3：A02 owner-issued guards 与 Linux 主机范围交付（2026-10-04）

实现提交 `fe0c0927418157cff83e2ebb41934d80cd12fae1`，前置文档基线
`75b0f32dcdc375d8caa5fe9983193400f1da1709`。九个 CLI 文件，284 insertions/
29 deletions。专题：[owner guards 与主机范围](validation/cli-owner-guards-2026-10-04.md)。
13处既有 guard 有限复核：11期限输入为 source-free InvalidInput，一空 accessibility
traversal idle-limit 为 source-free Timeout，一不可达 crash missing deadline 为
private CLI Precondition。没有伪造 OS code/parser/Elapsed。后者仅 CLI machine mapping
为 precondition，Core classifier 对该私有类型保持 Unknown，不要求 confirmation。
与 ForceRequired 共用32 source有界扫描，human fixed message/exit1/v1/effects 保持。

Accessibility 的第五次连续 idle：空集合失败，有已收集项正常结束，focus重置计数；
move_focus/limit/dedup/cleanup保留。七 new tests、两 strengthened old tests，包括
actual zero/overflow command/helper、empty/partial/reset policy、安全 envelope 和
31/32 depth。Secondary deadline guards 和不可达 crash state 仅 source review。
初次 seven tests PASS，但完整 CLI/first full 暴露 extra walk 使 cyclic-source 总访问
超过既有128预算（CLI695 PASS/one FAIL/one ignored，full exit1）；合并进原扫描后
17 output tests与 CLI696 PASS/one ignored，未放宽断言或制造旧实现 negative control。

修正 canonical full：session12108 terminal0/final PASS，Linux x86_64 Rust1.97.1/
MSRV1.80.1 Python3.12.13。命令
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`。
2115 Rust/13 Python，CLI696/Core1271/public38，zero FAIL/two existing ignored；
fmt/check/strict workspace Clippy/MSRV、66 features/four groups、7+7 doctests/three
snippets、C ABI、3.12 wheel/sdist、53 tooling/118 help/38 raw/five host/15 CLI/three
backup contracts PASS。22 device rows NOT_RUN；未重复 Chromium/installed3.9/3.13。
七 actual final-binary CLI cases：HID/display zero、RunXCTest zero/max、Companion/
Crash/OS Trace max；均 exit1/invalid_input/canonical phase/effects unknown、empty
stdout、raw capture clear、zero loopback usbmux trap connections、owned child cleanup。
Display overflow仍在setup后，只有helper实际验证；没有device流程/remote cleanup证明。
Source SHA256 `8299147db966f80a805ff1de460d4ad9e0a86c3dd70e6cd1dcc8fd4ec48819c3`；
ios debug SHA256 `933b59e94d44e209d525059bc11056da53369f340e29c7ba0da723a90adce78e`。
九源冻结至full terminal；Core/C/Python/deps byte exact。约19GiB余量，无clean/存储失败。
日志 `/tmp/ios-a02-owner-guards-`，首失败与修正证据均保留。

第三类 R01–R04、A01–A02 当前 Linux 主机范围交付。A02有限 acceptance matrix 覆盖
typed native/classifications、response/probe区分、stdout/stderr/stream/partial semantics、
mapped Python/C/HTTP及安全 machine输出；明确 legacy Unknown/void/zero/ALLOC profiles
保持兼容，不把 absence/metadata/NotAdvertised 推断为设备不支持。不是所有 legacy
错误都已转换，也不是平台或真机验收。118 capability/implementation/maturity 及历史
保持，仅增host证据；前置所有31任务/子项和待条件项继续保留。八文档/七 Markdown/
198 local links、whitespace、exact hashes/full/binary/capability audit PASS。

总体goal active，按第13节接续第四类 L01–L04/Q04。先 L01期限角色/失败后复用、取消/
清理主错误与有界队列；已定位 FPS unbounded channel/output early-return reader ownership
及 DTX caller cancellation/partial-frame reuse 边界，使用 InstallCoordination 契约作参考，
逐协议验证，不以错误归类替代生命周期保证。没有真实设备/daemon/credentials/Apple
操作、reference修改、push/publication、history rewrite 或maturity upgrade。

### 大类 4：L01 DTX 期限、取消与本地复用状态交付（2026-10-04）

实现提交 `940c4c673e7fe023503b97197777354c7f44e253`，源码基线
`fc204bf6a60edf15e0c54d47777091ec6034d425`，前置第三类文档
`c0b3681470d080361e91e00ab9db0e0852f615ab`。五源码/测试文件，694 insertions/
20 deletions；CLI只有既有测试迁移，production byte exact。契约
[生命周期与期限角色](lifecycle-contract.md)，专题
[DTX 本地主机验收](validation/dtx-lifecycle-2026-10-04.md)。

DtxConnection private guard 与实际 AsyncRead/Write progress：已完成 IO/codec error
或读取/写入任何字节后被取消，连接失效并释放 pending/live/outstanding/fragments/
memory accounting；后续公开 I/O 在 wire 前拒绝，is_usable 为明确本地字段。没有字节
进度的空闲取消仍可复用，unpolled future不改变state，完整成功仍可复用。Transport
仍 owned until client Drop，不声称remote shutdown/ACK/rollback/retry。
新增 channel/call/recv with_timeout 三入口共享 absolute cooperative budget，send/
ACK/unrelated/reply/reassembly不重置；零预算不poll/不改ID/不发bytes，overflowInput，
actual expiry原Elapsed保留。Legacy timing defaults不改，公开三variant DtxError未扩张。

CoreProfile quiet500ms slot和Accessibility initial300ms slots/idle None保留。三个
owner branch只在partial-frame expiry改为立即native Timeout，而非返回失效连接上的
表面成功。Channel/filter/serialization/limits不改，codec前半函数/constants/root/
classifier/deps/C/Python byte exact。Standalone read_dtx_frame仍由caller管理取消。

新增11 tests：八codec、一CoreProfile、一Accessibility、一external。Actual owned
duplex覆盖zero/overflow/unpolled、成功channel与correlated calls、旧method caller
timeout及late reply拒绝、partialheader/fragment、16-byte blocked send/ACK、EOF/
BadMagic、empty idle复用。Private constructed/warmed service clients验证start/idle/
initialflush和partialframe；不是完整device handshake/startup/cleanup。All-feature
10 internal/one external，minimumdtx八 internal/one external、strictCore/CLI PASS。
Fixture准备E0599/E0716和channel-sign后续等待已修；已定位test进程曾SIGTERM，并增
后续1s预算；不改变生产channel规则/断言，也无旧实现negative control。

第一次 full 的旧CPU fixture期待 EOF后 stop，CLI695 PASS/one FAIL/one ignored；迁移
为 count成功仍stop、EOF失效不写，保留原IO/source/有效JSON行和primary错误。
首full handle恢复时missing，finalFAIL/no activeprocess证明已停，rawexit未捕获。
第二full terminal1，Core1280 PASS/one FAIL/one ignored：既有AFC Debug forbids三位
129，可能与process-keyed correlation碰撞。单独test-only修正提交 `fc204bf` 保留
owned39bytes/Protocol/source检查，检查完整source表示；AFC production未改。

最终full session85615 terminal0/finalPASS，Linuxx86_64 Rust1.97.1/MSRV1.80.1/
Python3.12.13。命令
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`。
2126Rust/13Python，CLI696/Core1281/public39 PASS，zeroFAIL/two existingignored；
fmt/check/workspace strictClippy/MSRV、66 features/fourgroups、7+7doctests/three
snippets、CABI、3.12wheel/sdist、53tooling/118help/38raw/fivehost/15CLI/threebackup
contracts PASS。22device rows NOT_RUN，未重复Chromium/installed3.9/3.13。
Source SHA256 `e483b09329f51c4462d8f7de79d23b366b9ca3ca742dbad857e02fd5f602826d`；
ios debug SHA256 `88511ed0a9f8b7d4fe5c286c418f5593c697d1bd21bba185611baf9f201c1e3d`。
五源冻结至full结束，18→17GiB余量，无clean/storagefailure；日志 `/tmp/ios-l01-dtx-`。

本轮仅交付L01的DTX profile，L01/第四类/总体31goal active，所有子项及待条件保持。
118 capability/implementation/maturity与完整history保持，仅增host evidence。
八文档收尾：七Markdown/206 local links、whitespace、exactsource/binary/full logs与
capability catalog PASS。
下一已定位 FPS 无界channel、时间戳累积/窗口deadline starvation、output early-return
reader detach及stop/join budgets；继续其他协议/清理primary retention，再L02–L04/Q04。
没有真实device/daemon/credentials/Apple操作、reference edits、push/publication或historyrewrite。

### 大类 4：L01 FPS ownership、采样窗口与限时清理交付（2026-10-04）

实现提交 `dcec5a2032f2994ed45678cb838447607abee65b`，基线
`f3bbf4afcd523de6b6d94b7d73e5c898eebabecc`；五 source/test files，869 insertions/
119 deletions。原无界 event channel/background reader 改为 capture future 直接持有
session/read future；采样 ticks 不取消 partial DTX read，stop/已到期 absolute window
优先于 ready events。每 window 最多262,144 timestamps，下一条 typed CLI ResourceLimit，
不静默丢帧；additive iterator 避免另一 collection，legacy Vec parser/FPS calculator 保持。
[生命周期合同](lifecycle-contract.md)、[FPS 验收](validation/fps-lifecycle-2026-10-04.md)。

Setup 30s 覆盖 connect_instruments + public CoreProfile connect/start；外层 default
device resolver 不在其中。Stop 独立5s；实际 timeout 保留 Elapsed。主操作失败和 cleanup
失败同时发生时保留两个原对象、source 指向 primary；count/stop 成功但 cleanup 失败
现在返回失败。退出时取消 partial frame 可使 DTX unusable，stop 因而失败；whole-future
cancellation drop transport，不执行 async stop，没有 reader detach。Synchronous stdout/
parsing 不被 Tokio timer 硬中断，wire stop 不是 Apple ACK，保留真实验收边界。

新增12tests（9 CLI instruments/1 output/1 core/1 external），CLI新增10 PASS，output18
PASS且原 cyclic-source<=128 assertion 保留，full/minimum instruments iterator/consumer
各1 PASS。Owned public connect/start fixture 验证两 channel、machTimeInfo/config/start；
partial16-byte header 跨 sampling tick 后完整处理；10,000 ready frames + actual read counter
证实发出窗口时未清空backlog。真实 /dev/full ENOSPC 后保留上一行和 primary，64-byte
stop backpressure 保留 cleanup Elapsed；native EOF、cleanup-only timeout、operator stop、
injected native signal IO与caller cancellation EOF各有证据。Cap为prefilled边界/appender
测试，不声称full16MiB oversized wire/device workflow；signal IO非实际registration故障。

准备期 Bytes/Vec type、machTimeInfo ACK/Response与source-free IO get_ref assertion 修正；
首轮narrow2 PASS/7 FAIL，修正后8 PASS/1 FAIL发现backlogfixture错要求partial cancellation
后仍成功stop。改为sampler fairness/drop证据，production DTX invalidation未放宽。
最终 canonical full session88111 terminal0，validation/full PASS：2138 Rust/13 Python，
CLI706/Core1282/public40 PASS，0 FAIL/2既有ignored；fmt/check/strict workspace Clippy/
MSRV、66features/4groups、7+7doctests/3snips、CABI/3.12wheel/sdist、53tooling/118help/
38raw/5host/15CLI/3backup PASS。另 actual final binary + explicit UDID zero-window
InvalidInput/exit1、stdout0/trap0/childcleanup PASS；不外推defaultresolver。
Source SHA256 `3a597ba373e6999143308542a1c93a10d1b036cb28711ded50654263403920a5`；
ios debug SHA256 `ddd91930c66b50ed5dffb69c1e4f2083146b758982a0823b818acef5a44b7b16`。
Source frozen throughout full，约17GiB余量，无clean；日志 `/tmp/ios-l01-fps-`。

八文档收尾审计 PASS：七Markdown/213 local links、whitespace、exact source/binary/
full/narrow/binary trap logs 与 actual-help capability catalog；源码冻结至收尾。

22device rows NOT_RUN，未重跑Chromium/installed3.9/3.13，capability118正文/maturity/
history保持，只增host evidence。其他instruments production、root/errors/dependencies/
C/Python source保持。L01/第四类/完整31goal active；下一AFC transaction及H2/XPC流控/
回复关联的期限与取消状态，再L02–L04/Q04；平台/真机条件保留。无真实设备/credentials/
Apple操作、reference编辑、push/publication或historyrewrite。

### 大类 4：L01 AFC request/file ownership 与清理期限交付（2026-10-04）

实现提交 `c4b471d33b1260736a295e8718d3c47ee6562165`，基线
`24663c7327238461e4d67e8ddc6a7fdd02da1de0`；两 source/test files，966 insertions/
52 deletions。十二 request sites 同一send/flush/reply guard，optional setter配置absolute
per-request budget，default None维持legacy unbounded request timing，非whole-file/setup/
host-reader budget。Zero/overflow deadline无IO/ID advance，actualElapsed保留IO.get_ref；
cancelled zero-byte send可复用（已poll可跳过ID），unpolled不变。已有wire bytes、完成IO/
codec失败拒绝后续请求，late reply不被消费；counter exhaustion不wrap/panic。普通完整
Status业务错误可复用，short FileRefOpenResult失效，response packet-number policy未更改。
[生命周期合同](lifecycle-contract.md)、[AFC 验收](validation/afc-lifecycle-2026-10-04.md)。

Read_file与write_file_from_reader在返回operation failure/success后统一close，独立5s预算
与更短request budget夹限。Cleanup失败后client失效；只有一个失败时原variant/object保持，
两个失败通过existingIo拥有primary+cleanup并source指primary，category保持。Primary Status
在dual-failure path可迁移为外层Io，明确兼容迁移。Opened-file transfer在requests之间取消
也失效；Drop不await close、不spawn task，manual handles归caller，transport保持至Drop。
没有remote handle release/rollback/retry/Apple cleanup保证，也不硬中断CPU/host blocking。
Codec/256MiB caps/chunk/mode/path/wire、原tests/assertions、CLI/C/Python/root/errors/
classifier/dependencies保持。只读refs py38fbd227/go ced7e53d，原uv.lock/go.mod状态未改。

新增12tests（11 internal/1 external），full/minimum afc各12 PASS，AFC全31 PASS，
strict Core/CLI/fmt PASS。真实public requests、Status后复用、nativeEOF/badmagic/partial
header/body、16-byte send backpressure、zero-byte stalled adapter、十四public入口拒绝、
late frame仍未读、actual open/read/close与host native FileOpen IO、production独立5s cleanup
timer及transfer cancel between requests都有owned duplex证据；无fixture tasks。首narrow
7 PASS/4 FAIL是helper误从IO.source找Elapsed（实际IO.get_ref），修正只改helper，原timer
object保留。没有old-implementation negative control或device startup/filesystem证据。

Canonical full session55322 terminal0，validation/full PASS：2150 Rust/13 Python，
CLI706/Core1293/public41 PASS，0 FAIL/2既有ignored；fmt/check/workspace strict Clippy/
MSRV、66features/4groups、7+7doctests/3snips、CABI/3.12wheel/sdist、53tooling/118help/
38raw/5host/15CLI/3backup PASS。Source SHA256
`edf675e1b7eaa0a4c514ac1d8d9475f75010af2934ced87c5c714e4d556d1a17`；ios debug SHA256
`c506bf709ac1db3aca9adeada84d7bc6042fe5ffb42781764df918486481750c`。
Source冻结至full结束；余量约14GiB，无clean/storage failure，日志 `/tmp/ios-l01-afc-`。

八文档收尾审计 PASS：七Markdown/220 local links、whitespace、exact source/binary/
full/narrow/minimum/regression与refs状态、actual-help capability catalog；源码保持冻结。

22device rows NOT_RUN；Chromium/installed3.9/3.13未重跑。Capability118正文/maturity/
history保持，仅增host evidence。L01/第四类/完整31goal active；下一H2/XPC流控等待、
本地message consumption与call send/reply的取消/期限ownership，再其他服务清理/L02–L04/
Q04。设备/platform条件保留；没有设备/credentials/Apple操作、reference edits、push/
publication或historyrewrite。

### 大类 4：L01 H2 帧取消、流控等待与内部期限交付（2026-10-04）

本地 source `ddd3620f058c6c23649a083d209dceb41f0eb472`，baseline
`529f65e0dd81a7bfa86170cfd6ac3a986cf3b6b9`，分支
`feat/device-service-updates-20260927`。两 source/test 文件，578 insertions/9 deletions；
Linux x86_64、Rust1.97.1/MSRV1.80.1、Python3.12.13。细节与边界见
[H2 host gate](validation/h2-lifecycle-2026-10-04.md)、
[统一生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、[分类](error-contract.md)。

H2 read_stream/read_next_data_frame/buffer_next_data_stream/write_stream/open_stream/
finish_stream及borrowed finish_connect各自持有字节进度守卫；nested open不重置外层。
实际transport error/EOF或cancelled reported bytes使framer失效，清除retained H2 buffers/
DATA queue/byte accounting/stream-window集合；后续framer操作在IO前拒绝，需fresh
transport。Handle保留到Drop，borrowed transport不可绕过失效状态，Drop不await
shutdown/cleanup，不spawn reader/task。Zero-byte取消/expiry与unpolled保持可复用；
complete stream-local RST/EOF/GOAWAY isolation及既有connection Protocol政策保持。

Internal set_operation_timeout共享一次HEADERS/DATA/flush/flow-control的absolute budget；
zero在IO/state前返回source-free TimedOut，unrepresentable返回InvalidInput，实际
expiry在既有H2ErrorIo.get_ref保留Elapsed。H2 parent module私有；无新root export/
XpcClient option/frontend default，所有production callers仍None。这不是connect/
whole-XPC-call deadline。Cooperative timer不抢占同步工作。

新增八internal/一external测试，full九PASS、minimum dproxy八PASS，XPC regressions
102PASS。实际partial4/11B raw header/body、late完整帧保持未读、native EOF/BrokenPipe、
4/16B blocked HEADERS/DATA、注入0/2B flow windows及实际peer credit/DATA/partial
WINDOW_UPDATE、completeRST/END_STREAM的其他流复用均通过；public XpcClient完成
实际H2/RemoteXPC bootstrap后partial raw frame取消，后续recv/send拒绝。Private
framer/window injection不当成physical negotiation；concurrent peer exchange是owned
join/1s bound，无新增fixture tasks。

准备期首次external import私有H2路径编译失败，改为public XPC fixture，无export；
次轮7PASS/1FAIL因测试错误假设native read_exactEOF无customIOpayload，改查original
kind/OS-code，未改变原错误满足测试。首次full session57967 terminal1，在Rust阶段
CLI dispatch-file stack overflow/SIGABRT；inline async deadline wrapper放大nested
future。Source改为进入async wrapper前Box持有future，原CLI stack test/16KiB断言和
环境未放宽，相关测试恢复PASS，final narrow/minimum/regression/strictCoreCLIClippy/
fmt通过。无old-source negative control/benchmark；Linuxhost不代替Windows。

第11节最终 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session10807 terminal0，
validation/full PASS：2159Rust/13Python、0FAIL/two old ignored，CLI706/Core1301/
public42；fmt/check/strictworkspaceClippy/MSRV、66features/fourgroups、7+7docs/3snips、
CABI、3.12wheel/sdist、53tooling/118help/38raw/5host/15CLI/3backup合同均PASS。
Frozen source SHA256
`e7a60ac94d78611310df7e4fb326b6f7b98b481594707b1e67e15b442d7a3e20`；ios debug SHA256
`33da543e42db13bcc0454d2a3deaa6784834d6421f650498b17b61a6ff690d0c`。
Source冻结至full结束；wire constants/preface/bootstrap/raw decoder/builders/error enum/
全部旧H2tests byte-identical审计PASS，root/frontend/classifier/deps/refs未改；余量约
11GiB，无clean/storage failure，日志 `/tmp/ios-l01-h2-`。

八文档收尾审计PASS：七Markdown/229 local links、whitespace、exact source/binary/
full/narrow/minimum/regression/stack及refs状态，actual-help capability118目录；源码未改。

22device rows NOT_RUN；Chromium/installed3.9/3.13未重跑。Capability118正文/maturity/
history保持，仅增host evidence。L01/第四类/完整31goal active。下一上层XPC consumed
wrapper header/body、sent-call waiting-zero-reply、pending replies与stream/file ownership
的取消/期限；本轮H2guard不能替代这些logical boundaries，不能声明所有XpcClient
queued replies均拒绝或整个call共享预算。其他服务清理/L02–L04/Q04/platform/device
保持待条件；无设备/credentials/Apple操作、reference edits、push/publication/historyrewrite。

### 大类 4：L01 XPC message/call/stream/file ownership 交付（2026-10-04）

Source `09ed113e2d19489259a43c3a35de9e00caacb1a6`，baseline
`b58816f64cfe1871dd97b50eaf9b0dac00699ded`，分支
`feat/device-service-updates-20260927`。六source/test文件，1242 insertions/72 deletions；
Linux x86_64、Rust1.97.1/MSRV1.80.1、Python3.12.13。详细迁移与fixture边界见
[XPC host gate](validation/xpc-lifecycle-2026-10-04.md)、
[生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、[分类](error-contract.md)。

XpcClient新增is_usable、two call-with-timeout/three single-read-timeout entry；前者整次
send/flush/flow-control/unmatched/correlated reply共享absolute budget，后者明确idle
读取，legacy timing仍unbounded。Zero在IO/IDs前source-free TimedOut，unrepresentable
InvalidInput，实际expiry保留原Elapsed in XpcIo.get_ref。Timer不抢占同步encoding/
parsing/allocation。Guard跟踪已成功发送与已消费24B wrapper header，包括早先H2
buffer里的header；取消/失败在commit后或lowerH2invalid，清pending replies/memory
budget并失效H2，cached/wire后续均拒绝。Empty idle/unpolled保留复用；完整stream-local
失败在未commit时保留其他流，abandoned committed call需freshtransport。CounterMAX
不wrap/panic/IO，receive framing仍可用；getter不是peer liveness/每流availability。

Raw stream跨yield持有请求，send后Drop需freshtransport；next() poll取消保留stream
内部future。CoreDevice以原有finishStreaming parser显式完成guard，正常终止仍可复用。
Pasteboard有限PULL/SET/RESOLVE改用一次guarded uncorrelated send/nonempty reply，原
empty ACK过滤、business parser与timeout wrapper保持，正常回复及Service error可复用。
File sender从announced preamble到hostreader/chunks/sizecheck/FIN持有guard，read间隙
取消/错误也失效；成功FIN保留receivehalf。原twoGiB/64KiB/IDs/streammapping/wire保持，
无newfile/reader预算、async FIN/cleanup/task；transport保留到clientDrop，无remoteACK/
release/rollback/safe retry保证。

新增16tests：15internal/1external fullPASS；minimum tunnel11internal/1external PASS。
XPC117/Pasteboard30/CoreDevice10回归PASS，旧tests/assertions byte-identical。Owned
joins/1s bound，无新增fixturetasks。实际publicbootstrap/blocked4096Bsend/lateReply、
200ms总预算下peer125ms后drainsend再125ms后reply、已buffered24Bheader的emptybody
取消、rawpartial/nativeEOF/badwrapper清seededqueues、streamreset/counterinjection、
rawstream两yield间Drop、实际announce+hostIO/readercancel与content/FIN、CoreDevice
正常finish及PasteboardServiceError后secondPULL均通过。Private seams不是设备流量。

准备期两Vec/Bytes fixture错误、CoreDevice input应XpcValue而非IndexMap、minimum
缺CoreDevice gate、Clippy multipleboundlocations分别修正类型/feature条件/bound位置，
未放宽旧断言/生命周期policy。Finitecaller审查后在full前补normalcompletion适配，
无old-source negative control/benchmark。最终narrow/minimum/strictCoreCLIClippy/fmt
通过，原CLI stack预算测试及最终full保留，源有Box持有future，无detachedreader。

第11节首次 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session34637 terminal0，
validation/full PASS：2175Rust/13Python、0FAIL/two old ignored，CLI706/Core1316/
public43；fmt/check/strictworkspaceClippy/MSRV、66features/fourgroups、7+7docs/3snips、
CABI、3.12wheel/sdist、53tooling/118help/38raw/5host/15CLI/3backup合同均PASS。
Frozen source SHA256
`5742b3a91ffedae8a0978d52ee10049f9ae19a08669f384732e2e24d60237cfa`；ios debug SHA256
`5f4c39cf383a877365c38286a312b17788dee61746f8cd0c47b23757f7b37dd5`。
Source冻结至fullterminal；RSDbootstrap/bodyreader/wire、pendingbudget/queuehelper、
CoreDevicegrammar/Pasteboardparser未改，filemove移除marker/borrow调整后相等；H2仅
invalidatorvisibility，root/errorenum/codec/classifier/frontend/deps/refs保持。余量约
9GiB，无clean/storagefailure，日志 `/tmp/ios-l01-xpc-`。

八文档收尾审计PASS：七Markdown/238 local links、whitespace、准确source/binary/full/
narrow/minimum/regression/preparation与只读refs、actual-help capability118目录均通过；
源码未改，31任务/subtasks及既有evidence保留。

22device rows NOT_RUN；Chromium/installed3.9/3.13未重跑。Capability118正文/maturity/
history保持，仅增hostevidence。L01/第四类/完整31goal active，下一borrowedRSD
bootstrap/handshake、split send/recv/otherownertotals、其他protocol/service deadlines/
cleanup/orderedownership；L02–L04/Q04/platform/device条件继续。无设备/credentials/
Apple操作、reference edits、push/publication/historyrewrite。

### 大类 4：L01 borrowed RSD bootstrap/read ownership 交付（2026-10-04）

Source `85b8621f069b164d7da251a9531faf43b3456691`，baseline
`2088e739f04134e60341346cfdb1d4ca93e86f8d`，分支
`feat/device-service-updates-20260927`。单文件rsd.rs，543 insertions/2 deletions；
Linux x86_64、Rust1.97.1/MSRV1.80.1、Python3.12.13。详细边界见
[RSD host gate](validation/rsd-lifecycle-2026-10-04.md)、
[生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、[分类](error-contract.md)。

Borrowed初始化/queuedbootstrap在首个完整send后持有logicalguard，raw stream1/3
在消费24B wrapper后commit，包括早先H2buffer中的header；handshake在完整读取消息
后跨原有six-message非handshake过滤保持ownership。Commit后取消/返回错误或lowerH2
失效，使用原invalidator清H2 buffers/queues/accounting并拒绝复用；原error/source
不改，后续拒绝是existingH2-to-XPC Protocol。Empty idle/unpolled、成功完成及未commit
的完整stream-local失败保持framing/其他流。不是peer liveness/所有流availability/
remoteACK/release/rollback/safe retry；transport到Drop前仍由caller持有，无asynccleanup。

没有新publicentry/enum/frontend/deps/RSDtimer。现有dial/bootstrap/response/outer预算、
freshtransport重试顺序和singlewritehosthandshake保持；分别已完成的queue/send/read
不自动合成预算。既有ownedRSDattempt在取消/失败后丢弃transport，manualpairingbootstrap
使用受保护init，而其后directprotocol exchanges仍需owner规则。Future在awaitstate前
Box且directlyowned，五项旧CLI16KiB stack预算测试通过，无任务/reader/queue增加。

新增7internal tests，allfeatures/minimum tunnel各7PASS，XPC124回归PASS。Owned joins/
1s bounds，无fixture任务。实际duplex首次init完整发送后的callerCancel/Elapsed及late
reply、stream1/3已bufferedheader的zero-new-wire bodyCancel/Elapsed、精确duplexcapacity
使首条DATA写完而HEADERS3尚无byte时取消、skippedACK后取消、badwrapper/oversize、
RST在commit前后不同reuse、成功init/queue的原wire/宽松discardACK/handshake/followup
均覆盖。Cached stream5剩余byte在失效后拒绝；Drop前peer仍等待，Drop后EOF。原parser、
bootstrap/rawbody移除marker后byte相等，全部旧tests/assertions、XpcConnection和budgets
byte-identical。No old-source negative control/benchmark/device证据。

准备期firstnarrow session88918 terminal101 E0061，旧test调用原private读取入口；保留
其signature，把guard置于该入口并移body到inner，未修改/放宽旧tests。Secondnarrow
session19913 terminal0；minimum/regression/stack/strictCoreCLIClippy/fmt pipeline53566
terminal0，全部PASS。第11节首次 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session91844 terminal0，
validation/full PASS：2182Rust/13Python、0FAIL/two old ignored，CLI706/Core1323/
public43；workspacefmt/check/strictClippy/MSRV、66features/fourgroups、7+7docs/3snips、
CABI、3.12wheel/sdist、53tooling/118help/38raw/5host/15CLI/3backup合同均PASS。
Frozen source SHA256
`bedc6cff1eb4bbd50c5539416cc877c10a6f203644bf78d82677f1c6ff614341`；ios debug SHA256
`685fee93dad137cd7b89c0c66384539ca5742c67a0831cefe31062f8f9110027`。
Source冻结至fullterminal，余量约7.7GiB，无clean/storagefailure；日志
`/tmp/ios-l01-rsd-`。H2/codecs/errorenums/classifier/frontends/deps/refs未改。

八文档收尾审计PASS：七Markdown/247 local links、whitespace、准确testedcommit/
frozen source/binary、full/narrow/minimum/regression/stack/preparation与只读refs、
actual-help capability118目录均通过；源码未改，全部31任务/subtasks及history保留。

22device rows NOT_RUN，Chromium/installed3.9/3.13未重跑；Capability118正文/maturity/
history不改，仅增hostevidence。L01/第四类/完整31goal active；下一LockdownClient及
device侧borrowedLockdownsession、DeviceLink/plist、manualpairing后续、其他splitrequests/
owner总预算/cleanup/longstreams。L02–L04/Q04/platform/device条件保留。无设备/credentials/
Apple操作、reference edits、push/publication/historyrewrite。

### 大类 4：L01 Lockdown request/session-close ownership 交付（2026-10-04）

Source `79f750e78b4550909c440fc4697b592101878935`，baseline
`0f4c31fc99440e7698a78716db028c59dbb81cdd`，分支
`feat/device-service-updates-20260927`。三source/test文件，769 insertions/38 deletions；
Linux x86_64、Rust1.97.1/MSRV1.80.1、Python3.12.13。详细边界见
[Lockdown host gate](validation/lockdown-lifecycle-2026-10-04.md)、
[生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、[分类](error-contract.md)。

LockdownClient新增is_usable/set_request_timeout，defaultNone保持request timing
unbounded。Optional absolute预算覆盖encode/send/flush/replydecode，zero/overflow在IO前
拒绝，expiry保留原Elapsed in Io.get_ref。Query/value/startservice/activation统一guard，
bufferedhalves acceptedbytes也记录；取消/失败有progress或nativeIO/EOF失败则拒绝后续
cached/late replies。Local serialize failure在IO前可复用；完整business回复在原parser
之外保持legacy/preserving分类和reuse。Opaquehalves/buffers到Drop前仍持有，无queue/task。

StopSession独立5s写/flush预算，被更短requestbudget clamp；SID直到成功写才清除，
成功后clientclosed，其余requests Protocol，repeatstop localOk无IO。失败/取消有bytes
则SID保持uncertain、clientinvalid，不再假装repeatclose成功；emptywait expiry可保留
复用。成功write不代表remoteACK/release/rollback/retry。无automaticstop/asyncDrop或
primarycleanupaggregate，manualcaller需保留两result。Setupconstructor、rawborrowed
protocol/session helpers、device侧freshownedsession路径保持；未增加全操作/setup预算。

新增10tests：9internal/1external，最终allfeatures/defaultminimum各10PASS；Lockdown73
回归、旧CLI5项16KiB stack、strictCoreCLIClippy/fmt PASS。实际TLS QueryType/StartSession/
upgrade后fullysentGetValue expiry+latecomplete reply+无secondrequest、duplex部分reply/迟到
reply拒绝、512B背压下4096B请求（peer125ms后读、再125ms后完整reply）共享200ms期限、
privateprefilled64B emptywrite vs accepted64B、cached4Blen/bodywait取消、完整MissingValue/
InvalidService后successreuse、nativeEOF/plistdecode/oversize、stop正常/idempotent/partial
callerCancel/IO/shortruleexpiry及实际default5s expiry均覆盖。一般ownedfixture1s bound，
TLS2s/defaultcleanup6s；无新增fixturetasks。Publicconsumer只编译getter/setter、八项Send
future及three-variant exhaustive match；TLSruntime内部经publicconstructor，fakecert/
no-client-auth不是Apple认证证据。No old-source negative control/benchmark/device证据。

首次narrow14654 terminal0八tests，fixture unusedmut修正后32392 terminal0九internal/
oneexternal；86481 terminal0 narrowpipeline。之后补stopcallerCancel，14991 terminal0；
总预算夹具从晚到partialprefix改为512B可容完整reply，10951 terminal0最终10；临时audit
排除publicfixture时endoffset修正，源码未因此修改。旧assertions/输入/policy保持，六旧
clientfixture仅补默认privateState；原请求envelopes/responseparsers机械合并前后相等，
rawframing4MiB/TLS/session/device/root/errorenum/classifier/frontends/deps/refs未改。

第11节首次 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session36426 terminal0，
validation/full PASS：2192Rust/13Python、0FAIL/two old ignored，CLI706/Core1332/
public44；workspacefmt/check/strictClippy/MSRV、66features/fourgroups、7+7docs/3snips、
CABI、3.12wheel/sdist、53tooling/118help/38raw/5host/15CLI/3backup合同均PASS。
最终defaultminimum14987 terminal0十tests再次验证最后fixture，未再改源码。
Frozen source SHA256
`a1a3730a55e5d55bc6ba3b86778a65a46b011a268c9a980806f304c98dee4a3f`；ios debug SHA256
`82571798d8abdfe8c6dada926616ad15ca760f4e2d02388ee468709ef378e755`。
Source冻结至full/minimumterminal，余量约7.4GiB，无clean/storagefailure，日志
`/tmp/ios-l01-lockdown-`。

八文档收尾审计PASS：七Markdown/256 local links、whitespace、准确testedcommit/
frozen source/binary、full/finalnarrow/minimum/regression/stack/preparation与只读refs、
actual-help capability118目录通过。临时文档审计匹配sourceaudit日志文字修正，
未改源码/断言；全部31任务/subtasks和既有history保留。

22device rows NOT_RUN，Chromium/installed3.9/3.13未重跑；Capability118正文/maturity/
history不改，仅增hostevidence。L01/第四类/完整31goal active；下一DeviceLink/plist及
backup rawownership、borrowedrawsession调用方、setup/service/多request总预算、其他清理/
longstreams与manualpairing后续；L02–L04/Q04/platform/device条件保留。无设备/credentials/
Apple操作、reference edits、push/publication/historyrewrite。

### 大类 4：L01 DeviceLink message/handshake ownership 交付（2026-10-04）

受测实现 `3db5c1835ad73bf881c1c6a334674bc566cfe641`，基线
`a3d9a7cbd5d4cd4a5b247861b724469da0f2102f`；四个源码/测试文件，808 insertions /
24 deletions。Linux x86_64，Rust1.97.1/MSRV1.80.1、Python3.12.13；主机增量，
不升级设备 maturity。索引：[DeviceLink host gate](validation/device-link-lifecycle-2026-10-04.md)、
[生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、[分类](error-contract.md)。

DeviceLinkClient 新增 is_usable/set_operation_timeout/call_process_message。
DefaultNone 保留 unbounded timing/idle reads；absolute budget 覆盖单消息、完整版本
握手或 process send/reply，分开完成的 send/read 不自动共享 owner。Zero/overflow 在 IO
前拒绝，expiry 保留原 Elapsed in Io.get_ref；同步 encode/decode 仍 cooperative。
Guard 记录 accepted bytes（含已有 buffered transport）；取消/失败有 progress、native
IO/EOF 或 frame decode failure 后拒绝后续 cached/late reply，empty idle/write wait、
unpolled 和 local pre-IO failure 可保留状态。完整 process envelope 的原 parser/reuse
policy 保留；失败握手在消费消息后 invalid。没有新增 queue，opaque stream 到 Drop
前仍持有，boxed helper 直接 owned、无后台 task。

Disconnect 独立五秒 write/flush cap，被更短 configured budget clamp；MAX cleanup
也 clamp。成功后 local closed，其他操作及 repeat disconnect 返回既有 Protocol。
取消/失败已有 bytes 需新 transport，成功写不代表 remote ACK/release/rollback/retry；
无 automatic disconnect、async Drop 或 primary/cleanup aggregate。Public stream_mut /
into_inner 签名保留，raw IO 绕过跟踪且 caller-owned，不修复 invalid framing。

旧备份 EOF fixture 明确迁移：DeviceLink native EOF 已 invalid，不再 poll gated
disconnect writer；直接保留原 Mobilebackup2Error::DeviceLink::Io UnexpectedEof，检查
native source、无 issued timeout、unusable 和零 cleanup writes。此前预期 cleanup
timeout 包住 EOF。有效连接上的 business/success/pending cleanup-timeout 断言保持。
MobileBackup2 生产/raw transfer、screenshot/raw helpers、4MiB plist codec/limits、
原 envelopes/parser/error mapping、frontends/deps 不变；未宣称全备份会话安全。

新增十二 tests：十一 internal/一 external，最终 full/no-default minimum 各十二
PASS；DeviceLink14、backup96 PASS/一既有 ignore、旧 CLI 五项16KiB stack、strict
Core/CLI Clippy/fmt PASS。覆盖七入口 zero/unpolled、MAX、真实拒绝序列化/oversize、
empty idle cancel/expiry vs six-byte partial receive、cached four-byte header、prefilled
64B empty write vs exact accepted prefix、完整 process send 后 caller cancel 和 late
reply 拒绝、4096B request/512B背压（125ms drain +125ms complete reply）共享200ms、
握手125ms version +125ms ready 共享200ms及 VersionsOk 后 caller cancel、完整 process
envelope 错误后正常复用、native EOF/BrokenPipe/bad plist/oversize/wrong handshake、
disconnect exact close/partial cancel/short expiry/真实 default5s cleanup expiry。
Owned pinned futures/joins，交互和拒绝等待1s bound，defaultcleanup6s；无新 fixture
tasks。External 只检查四类签名、七项 Send futures 和 exhaustive 三 error variants，
无真机 IO；无 old-source negative control/benchmark/other-platform acceptance。

首次 narrow83458 terminal101：新测试 E0283 key inference/E0716 temporary lifetime，
修正后61628 terminal0十二PASS。加 fixture wait bounds 后 pipeline97516在旧 EOF cleanup
断言停101，95 backup PASS/一FAIL/一既有ignore；按上述显式合同迁移。最终 pipeline
97982 terminal0：minimum12、DeviceLink14、backup96/一ignore、stack5、Clippy/fmt。
审查确认原 DeviceLink/public consumer 字节及其余旧 backup bodies/inputs 保留，
version body 在机械 inner rename 后相等，process parser/error mapping 精确不变；
reference HEAD/status 保持只读。

第11节首次 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session24485 terminal0，
validation/full PASS：2204Rust/13Python，0FAIL/two old ignored，CLI706/Core1343/
public45；workspace fmt/check/strictClippy/MSRV、66features/fourgroups、7+7docs/
3snips、CABI、3.12wheel/sdist/install、53tooling/118help/38raw/5host/15CLI/3backup
合同全部PASS。Frozen source SHA256
`7b7cfbce2b68fd3e1a08ad12c74f9d08bdef6eb62c6e1dffd2fa8abd07493bc2`；ios debug SHA256
`d2e81b9165a545eae25d97f8380719b948cc958eff43089d7f59806170398539`。
Source 冻结至 full/minimum terminal；full 前余量约7.2GiB、sdist期间约6.3GiB，无
clean/storagefailure。日志/audits `/tmp/ios-l01-device-link-`，结果持久化本节及专题。

八文档收尾审计 PASS：七 Markdown/266 local links、准确 testedcommit/frozen source/
binary/full/minimum/regression/stack/preparation 与只读 reference HEAD/status、whitespace
及 exact八文件范围；capability源目录及 actual CLI help118通过。首次插入位置匹配了
较早的相同结尾，已将本记录移至历史末尾；audit确认除L01状态行外旧prefix精确保留，
全部31任务/subtasks/既有记录不丢失。产品源码未再改动。

22device rows NOT_RUN，Chromium/installed3.9/3.13未重跑；Capability118正文/maturity/
history 保持，仅新增本次 host evidence。L01/第四类/完整31goal active；下一步
MobileBackup2 Hello/command/host-work gaps、raw upload/download 和完整操作 cleanup。
Raw borrowed sessions、setup/service/多请求 totals、其他服务/longstreams/manual-pairing
ownership、blocking-worker/resource bounds、L02–L04/Q04/platform/device 条件保留。
取消 host spawn_blocking 的 await 不等于 worker 已停止；未操作设备/credentials/Apple、
未改 reference、push/publication/history。

### 大类 4：L01 MobileBackup2 complete operation ownership 交付（2026-10-04）

受测实现 `d2828b802e697ff4d1069744e89d7a1eb31214c7`，基线
`65cf7384eafac8fb36ecd0cd09b105faaf119830`；明确七个授权源码/测试文件，
1280 insertions/77 deletions。Linux x86_64、Rust1.97.1/MSRV1.80.1、项目
Python3.12.13。主机交付，不升级 device maturity。
索引：[MobileBackup2 gate](validation/backup-lifecycle-2026-10-04.md)、
[生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、
[错误](error-contract.md)、[备份](backup-contract.md)。

Mobilebackup2Client 新增 is_usable/set_operation_timeout；一个 absolute budget
覆盖 version/Hello、command、host work、raw upload/download 和 cleanup。
Ordinary defaultNone 保留 unbounded timing，Unback/Extract 保留300s ceiling，
更短 configured budget clamp。Zero/overflow 在 IO/host work 前拒绝，actual expiry
保留 Elapsed；同步 encode/decode 和已运行 blocking worker 仍为 cooperative 边界。
Private accepted-byte transport/operation guard 拒绝未完成有进度的取消/失败及 native
IO/EOF 后复用，九个 public operations 不再读取 late reply；释放私有 diagnostic/
filter/discarded-path state。Empty idle、unpolled/local pre-IO failure 可保留状态。
Boxed future 直接 owned，无新增 background tasks/queues。

完整 DLMessageProcessMessage 在业务 dictionary/ErrorCode 校验前标 terminal，
保持既有 bounded disconnect。Cleanup 共享 total deadline 和 DeviceLink5s cap；
operation expiry 不再启动 late disconnect，abandoned raw transfer 不插入 control。
完整 terminal reply 后预期 BrokenPipe/ConnectionAborted/ConnectionReset/
NotConnected/UnexpectedEof 保留旧 completion policy，但 local transport unusable。
其他 cleanup error 包括成功后的失败均报告。新增 opaque SessionCleanupFailure /
cleanup_failure，32 typed sources（含 IO payload）有界查找，借用两个原 error。
Dual failure 用既有 Io，source/category 跟随 primary；cleanup await 期间 primary
仍 owned，outer expiry drop body 不丢失。这明确替换旧 cleanup-timeout-first 分类。
四个 public error variants 保留；无 asyncDrop/remoteACK/rollback/retry/workerstop 保证。

十二 new tests（十一 internal/一 external），最终 full/no-default minimum 各十二
PASS；backup lib107 PASS/一既有 ignore、integration11、DeviceLink14、旧 CLI 五项
16KiB stack、backup CLI21、strictCore/CLI Clippy/fmt PASS。覆盖 preflight/idle、
version后zero-byte Hello wait、sent/fragmented late reply、negotiation/host/final
reply total、raw两方向取消、hostwait与hostcleanup expiry、native/actual5s cleanup
双分支、cycle/context lookup、public九个Send/exhaustive四variant。
Controlled worker release 不证明任意 spawn_blocking stop/join；无 old-source negative
control、benchmark、真机或其他平台验收。

交接旧回归95PASS/1FAIL/1ignored，terminal标记已提前；最新交接source仅fmt/diff。
本次 narrow68597 terminal101 为测试 E0283，显式 plist::Value 修正；74098 terminal0
十二PASS，lib89356 terminal0 107PASS/一ignore。Pipeline11316 在 explicit integration
3PASS/8FAIL 停101：终止reply后的预期断连被新 checked cleanup 报错。恢复原准确
suppressible IO-kind policy，未改 integration/session 断言；最终 pipeline82468
terminal0 全部上述 narrow/minimum/regression/CLI/Clippy/fmt。源码审计确认原 transfer/
host-command tail 除已审查terminal marker外字节保持，Hello parser、旧session/
integration/publicconsumer保持；旧 native fixture count 保持，显式迁移 no-late-
disconnect 与 primary优先。DeviceLink仅private getter；protocol/host_fs/manifest/
root/enums/classifier/frontends/deps 和只读refs HEAD/status保持。

第11节 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session1410 terminal0，
validation/full PASS：2216Rust/13Python、0FAIL/two existing ignored，CLI706/
Core1354/public46。Rust/Python拆分；workspacefmt/check/strictClippy/MSRV、
66features/fourgroups、7+7docs/3snips、CABI、3.12wheel/sdist/install、53tooling/
118actual-help/38raw/5host/15CLI/3backup合同全部PASS。Frozen source SHA256
`64e0009837d0f74c146897ab3f3e01312b088659f83d23efcdcd7d10fb61d36c`；ios debug SHA256
`235f77ff5cfd61afa1b3838c0b048fe929a499800f8a4c71d2f4cfa0027a45c1`。
Source冻结至完整gate和实现commit；full前约6.0GiB/后4.2GiB，无clean/storagefailure。
本地日志/audits `/tmp/ios-l01-backup-lifecycle-`，结果保存在本节及专题。

九文档收尾审计PASS：八Markdown/280 local links，testedcommit/frozen source/binary、
full/narrow/minimum/regression/CLI/preparation及只读refs一致；Capability118源目录及
actual CLI help检查PASS，正文/maturity/history仅追加hostevidence。全部31任务/
subtasks/历史精确保留，除L01overview行外旧roadmap prefix相等，新记录位于末尾。
文档收尾未改产品source。

总体仍9项完成、11项未开始、11项进行中/待验收/待条件；完整31任务及全部subtasks
保留。L01/第四类/总体goal active；下一独立owner NotificationProxy framing/idle/
subscription/shutdown，再继续其他协议/cleanup、borrowedrawsessions、setup/service/
多request总预算、longstreams/manualpairing、blockingworker/resources、L02–L04/Q04
和第五至九类。22device rows NOT_RUN；Chromium/installed3.9/3.13未重跑，Capability118
正文/maturity/history保持，仅增hostevidence。平台/设备/credentials/协议条件如实保留，
不猜协议、不把主机验收当真机。无设备/Apple操作、reference edits、push/publication/
history rewrite。

### 大类 4：L01 NotificationProxy frame/subscription ownership 交付（2026-10-04）

受测实现 `db732437a945c75096d05f67b9e2063838756c97`，基线
`dd1090efcb58623c776aff51e10d3598321ba895`；五个源码/测试文件，849 insertions/
51 deletions。Linux x86_64、Rust1.97.1/MSRV1.80.1、项目Python3.12.13。
专题：[NotificationProxy gate](validation/notification-lifecycle-2026-10-04.md)、
[生命周期](lifecycle-contract.md)、[兼容](public-contract.md)、[错误](error-contract.md)。
新增 is_usable/set_operation_timeout，defaultNone保留unbounded send/idle；显式wait
取更短configured budget，wait_for包含Observe写入，unrelated events不重置absolute
deadline。Zero用既有source-freeTimeout在IO前拒绝（next_event旧zero-issued行为显式
迁移），ordinary overflow为IoInvalidInput；actual expiry保留Elapsed，ready完整event
耗尽预算时poll已到期Tokio timer，不再读取下一帧。同步codec仍cooperative。

每帧accepted-byte guard覆盖buffered IO；partial cancel/expiry、nativeIO/EOF/typed
codec failure清理订阅names、七个入口拒绝late/unread复用。已完成one-way subscribe后
empty idle cancel/expiry保留订阅及合法late events；完整business-shape error保持parser/
reuse。ProxyDeath保留原event/error并closed；Shutdown5s cap/更短configured clamp，
MAX安全clamp，成功closed/repeat及后续Protocol；empty blocked close可复用，partial
不可。无ACK/asyncDrop/automaticcleanup/newqueue/task；opaque stream到Drop仍owned。
Outgoing length checked u32 conversion是source-only极端边界证据。

十三new tests（十二internal/一external）最终all-feature/no-default notificationproxy
各十三PASS，notification22、CLI8、旧stack5、strictClippy/fmt PASS。覆盖七入口
zero/unpolled/MAX、empty idle订阅复用、2/4/6B partial和warmBufStream header、prefill64B
empty vs exactacceptedpostprefix、512B背压125ms写+125msreply共享200ms、complete
subscribe后callerCancel、businessshape后reuse/ProxyDeath terminal、nativeEOF/BrokenPipe/
badplist/typedName/oversize、Shutdown精确wire/MAX/emptypartialcancel/实际default5s，以及
20unrelatedevents不重启configured100ms。Public编译七Send/exhaustive5errors/两events；
owned futures/joins和bounded等待，无old-source negativecontrol/benchmark/device验收。

首次narrow22673 terminal101为新测试E0283 key inference，显式String修正；7128
terminal101，11PASS/1FAIL揭示ready-event loop无Elapsed；生产改为poll已到期真实timer。
最终91032 terminal0十三PASS，pipeline73242 terminal0全narrow范围。原coretests/serde/
两businessparser/incomingframing机械trackedstream替换后相等，4MiB/typeddecode保留；
原publicconsumer字节保留。CLIproduction精确保持，旧ProxyDeath fixture加closed拒绝，
再freshconnection执行原idle-zero-issued/EOF断言；root/enums/其他协议/deps/refs保持。

第11节 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session80002 terminal0，
validation/full PASS：2229Rust/13Python、0FAIL/two existing ignored，CLI706/Core1366/
public47。Rust/Python拆分；workspacefmt/check/strictClippy/MSRV、66features/fourgroups、
7+7docs/3snips、CABI、3.12wheel/sdist/install、53tooling/118help/38raw/5host/15CLI/
3backup合同均PASS。Frozen source SHA256
`2f0c36d901792fcff6e1efa409876f1ce59e55edfb3edf69e6db21c3e007fd38`；ios debug SHA256
`67ef78da06e6086178032c2b2906e1f50ec8e623c53c40a3a3d296ebea4cf4fa`。
Source冻结至full/commit；full前约3.6GiB/后3.5GiB，无clean/storagefailure；日志/audits
`/tmp/ios-l01-notification-`，结果持久化于本节/专题。

八文档收尾审计PASS：七Markdown/285 local links、准确testedcommit/frozen source/binary/
full/minimum/regression/CLI/preparation/只读refs一致；Capability118源目录与actualhelp
PASS、原正文/maturity/history保留。所有31任务/subtasks/历史prefix保持，只有L01行
改动及末尾交付追加，文档期间产品源码未变。

总体9complete/11未开始/11进行中或待验收/条件不变，全部31任务/subtasks/history保留。
L01/class4/总体goal active；下一CLI notification setup/total/signal/cleanup owner，当前
错误分支skip shutdown且成功后suppressed cleanup尚未验收；其他protocol/cleanup/
borrowedrawsessions/setup/multi-requesttotals/longstreams/manualpairing/blockingresources、
L02–L04/Q04及第五至九类继续。Capability118正文/maturity/history不改，只增hostevidence；
22device NOT_RUN，Chromium/installed3.9/3.13未重跑；平台/设备/credentials/协议条件保留。
无设备/Apple操作、refs修改、push/publication/historyrewrite。

### 大类 4：L01 notification CLI returned-error cleanup 交付（2026-10-04）

受测实现 `9b2c488ec9052d2899de91a68622b00be207446c`，基线
`a8917656824347914d9affc51b42592f97071f03`；两个CLI源码/测试文件，450 insertions/
93 deletions。Linux x86_64、Rust1.97.1/MSRV1.80.1、项目Python3.12.13。
专题：[notification CLI cleanup gate](validation/notification-cli-cleanup-2026-10-04.md)，
对应[lifecycle](lifecycle-contract.md)/[兼容](public-contract.md)/[errors](error-contract.md)。

Command body直接owned Box完成后持有Result再cleanup；仍usable的连接在returned success/
failure（含output/render）后尝试core独立5s Shutdown。Invalid/ProxyDeath不发unsafe
control，只在commandDrop释放transport；未尝试不制造secondary error也不代表remote
release。成功后cleanup失败现在报告，旧行为suppressed；已产生output/device effects
不回滚/不授权retry。Single primary原对象保持；dual保留两个原error，private aggregate
source/category跟随primary，Display静态/Debug仅classes；无task/queue/asyncDrop。
Setup/commandtimeouts/signal、parameters/help/事件选择/成功输出及core APIs/protocol保持。

九new helper-owned tests，固定narrow63198 terminal0九PASS；pipeline85220 terminal0
minimumCLI all-target/no-default check、notificationCLI17、旧16KiB stack5、strictClippy/
fmt PASS。覆盖exactpost+Shutdown+DropEOF、primary permission/context不丢、success+
nativeBrokenPipe、dualnative/shortissuedcleanup expiry（primaryclass/Elapsed分支）、
actualdefault5s success-cleanup failure（6s fixturebound）、partialread/ProxyDeath no late
write、ownedfuture取消DropEOF、安全v1/Display/Debug。其他wire等待1s有界；不是实际
deviceCLI/stdoutfault/OSCtrlC验收，无negativecontrol/benchmark/platform证据。
Sourceaudit旧match tokens/setup/parameters/output在fmt空白及唯一tuple-pattern trailing
comma归一后相等，旧streamhelper/renderer/tests字节相等；core/root/APIs/deps/refs未改。
首narrow75043 terminal101为测试E0599误用ErrorClass名，改existingPermissionDenied及
v1 error.class后通过；temporaryaudit的fmtcomma归一修正未改产品source/断言。

第11节 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session78503 terminal0，
validation/full PASS：2238Rust/13Python、0FAIL/two existing ignored，CLI715/Core1366/
public47。Rust/Python拆分；workspacefmt/check/strictClippy/MSRV、66features/fourgroups、
7+7docs/3snips、CABI、3.12wheel/sdist/install、53tooling/118help/38raw/5host/15CLI/
3backup合同均PASS。Frozen source SHA256
`e59de42bd588c30c6507c977745398e5ebc9636179a17af2d490218467b9d3e9`；ios debug SHA256
`56e8c8ae91f7b9c4887aa647a4ddc5268b80562722c3e79b937ed0703971377d`。
Source冻结至full/commit；narrow期间约4.0GiB/full后2.6GiB，无clean/storagefailure。
日志/audits `/tmp/ios-l01-notification-cli-cleanup-`，结果持久化本节/专题。

八文档收尾审计PASS：七Markdown/295 local links、testedcommit/source/binary、full/
narrow/preparation/只读refs一致；Capability118源目录/actualCLIhelp PASS，正文/
maturity/history保持仅追加hostevidence。全部31任务/subtasks/historyprefix精确保持，
仅L01overview行改动及末尾交付追加；文档期间产品source不变。

全31任务/subtasks/history保留，总览9完成/11未开始/11进行中或待验收/条件不变。
L01/class4/overallgoal active；下一notification CLI checkedsetup/finite totals/signal，
随后其他L01协议/cleanup/borrowedrawsessions/multi-requesttotals/longstreams/manualpairing/
blockingresources、L02–L04/Q04及第五至九类。Capability118正文/maturity/history保持，
仅追加hostevidence；22device NOT_RUN，Chromium/installed3.9/3.13未重跑，真实设备/
平台/credentials/协议条件保持。无设备/Apple操作/refs修改/subagents/push/publication/
historyrewrite。

### 大类 4：L01 notification CLI discovery/total/signal ownership 交付（2026-10-04）

受测实现 `e3ab42b8e1d47155edd9e0ffd08c668b5e3a41ec`，基线
`7e901d6f8c38b8f856797c1a453c686d1c5e626f`；六CLI/source/verifier测试文件，
1295 insertions/141 deletions。Linux x86_64、Rust1.97.1/MSRV1.80.1、项目Python3.12.13。
专题：[notification CLI control gate](validation/notification-cli-control-2026-10-04.md)，
对应[lifecycle](lifecycle-contract.md)/[兼容](public-contract.md)/[errors](error-contract.md)。

Finite zero/MAX在任何explicit/default设备选择前InvalidInput；默认ListDevices、device/
serviceconnect共享checked30s setup，一个pinnedCtrlC贯穿setup/capture，device owner
保留至cleanup。Post及完整subscriptionstage另有30s cap；Roundtrip/Wait/Springboard/
StreamSome总期限从第一subscription写入前开始，覆盖observe/post/全部subscription/
receive/output；setup前置、cleanup独立5s。Observe保留每事件idle，StreamNone无限期。
Ready stop/到期优先；完整frame/output边界yield防ready traffic饿死control，不tick取消
partialread。Stream/ObserveCtrlC及Stream总到期正常结束（仍可能cleanup失败）；finite/
setupCtrlC保留OperationCancelled、实际setup/send/finite到期保留真实Elapsed。Stream旧
idle到期失败/事件间到期成功现统一normalstop，是显式迁移。主错误/dualcleanup保留。
Partialinvalid/ProxyDeath不lateShutdown，无task/queue/asyncDrop/ACK/rollback/retry。

十九newRust fixtures最终pipeline76076 terminal0：narrow19、notificationCLI36、minimum
CLI all-target/no-default check、旧16KiB stack5、strictCore/CLIClippy/fmt PASS；Python
verifier11（三new含contact/private诊断negativecontrols）及实际CLI39 PASS。新24case：
五命令×zero/MAX×explicit/default共20JSON，加两human，22均0deviceconnections；有效
emptydefault只1ListDevices/0extra；Linux真实SIGINT仅发送自有CLI childPID，阻塞于
syntheticListDevices，cancelled/notify/exit1、peerEOF/ownedprocesscleanup PASS。实际39
rawhits0/完整capture，无物理设备/service/pairing/credentials/Apple操作。

准备阶段 `cargo clean -p ios-cli --profile dev` terminal0，删除4238构建缓存文件/
10.9GiB，free由2.6至约12GiB；此时无构建进程。首expanded37CLI检查35PASS/2humanFAIL，
因verifier误期望human label；修正为现有io/InvalidInput后37PASS，后续扩展至39PASS。
产品formatter未改。最新源码冻结后再full，无运行中编辑/重复验证。

第11节 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session13402 terminal0，
validation/full PASS：2257Rust/13Python、0FAIL/two existing ignored，CLI734/Core1366/
public47。Rust/Python拆分；workspacefmt/check/strictClippy/MSRV、66features/fourgroups、
7+7docs/3snips、CABI、3.12wheel/sdist/install、56tooling/118help/38raw/5host/39CLI/
3backup合同均PASS。Frozen source SHA256
`5329b8ccbc98417797eb15f0ce3a5ab3a1ee01e3368d27a04e920ec4e70531c2`；ios debug SHA256
`50f09b7fb0fe78eca75e945485e4d3a09b231f61ce08dae0e1e5023dbcc7ef64`。
Source冻结至full/commit，full后free4.3GiB，无storagefailure。源码audit旧clap/help、
cleanup/streamhelper/renderer/tests、其他main与原Python测试字节相等；core/API/enums/
protocol/deps/refs保持。日志/audits `/tmp/ios-l01-notification-cli-control-`。

八文档收尾审计PASS：七Markdown/305 local links，testedcommit/frozen source/binary/
full/narrow/preparation/只读refs一致；Capability118源目录及actualCLIhelp PASS，正文/
maturity/既有evidence保持。全部31任务/subtasks/historyprefix精确保持，仅L01overview
行和末尾交付追加；文档期间产品source未改。

全部31任务/subtasks/history保持，总览9完成/11未开始/11进行中或待验收/条件不变。
L01/class4/overallgoal active；下一Companion共享连接/request/listener/forwardingcleanup
owner，随后其他L01协议/rawborrowedsessions/multi-requesttotals/longstreams/manualpairing/
blockingresources、L02–L04/Q04及第五至九类。Capability118正文/maturity/history保持，仅
追加hostevidence；22device NOT_RUN，Chromium/installed3.9/3.13未重跑。Cooperative
timers不preempt同步encode/decode/output或已运行blockingworker；injectedwriter不是OS
stdoutfault，hostSIGINT不代替device信号/cleanup验收。平台/设备/凭据/协议条件继续；
无referencecheckout编辑/subagents/push/publication/historyrewrite。

### 大类 4：L01 Companion shared connection/listener/forwarding ownership 交付（2026-10-04）

受测实现 `7a0ed63d99f04d99971004b177dd68e1cdf3b3bb`，基线
`976f58c2f03586467e9031be49d1ffab678a67a7`；四core源码/测试文件，918 insertions/
62 deletions。Linux x86_64、Rust1.97.1/MSRV1.80.1、项目Python3.12.13。
专题：[Companion gate](validation/companion-lifecycle-2026-10-04.md)，对应
[lifecycle](lifecycle-contract.md)/[兼容](public-contract.md)/[errors](error-contract.md)。

共享Atomic localstate贯穿client/listener/forwardinghandles；添加is_usable，只表示local
admission。Request默认10s，checked absolute budget覆盖mutexwait/send/flush/reply；zero
pre-I/O legacyTimeout(Duration)、overflow IoInvalidInput、实际expiry保留原Elapsed/
durationwrapper。取得mutex后再查state，阻止已排队owner消费abandonedrequest迟到reply。
Accepted bytes/committedrequest取消、nativeEOF/IO/framecodec失败invalidate所有shared
owners；emptyblockedwrite/lockwait/unpolled/pre-I/O错误可保reuse。Complete business回复
guard外原parser保持。Opaque stream/buffers至所有sharedownersDrop才释放。

Listener保存ownedpending read跨cancellednextpoll，含partialheader。Actual idle仍terminal
（含empty），现在立即释放pending/mutex并invalidate sharedstate；wholelistenerDrop也
invalid，防另一owner把subscriptionevent当reply。FinishedreadNone保持；active zero/MAX
不poll/finish。Client/handle/既有Drop stop独立5s或shorterbudget，MAX安全clamp；已完成
successfulstop仍cached/idempotent，laterzero/MAX不重发。Completebusinessfailure可保reuse；
abandoned/native/codec需freshtransport。既有Droptask依然boundedbest-effort，stopped/
invalid/zero不spawn、锁后查state；completebusinessfailed explicitstop仍可能follow-upDrop。
不宣称awaitedjoin/primaryaggregate/remoteACK/cleanup/saferetry，CLIowner/disarm另待。

十六internal/一external新测试：pipeline65141 terminal0 full/minimum各17、Companion32/
CLI7/minimumCLI all-target-no-default/strictClippy/fmt PASS。初stackfilter错误匹配0，
不可算PASS，纠正fits_within_windows_main_thread_budget后5PASS（stack-corrected.log）。
覆盖200ms跨125ms send+125msreply、empty/partial/committedcancel、late/bufferedreply、
锁后sharedinvalid/六client入口reject、nativeEOF/BrokenPipe/typedbadplist/oversize、
businessreuse/cachedstop、actualexplicit及既有Drop5sexpiry、listenerpartialheldnext/
terminalrelease/wholeDropEOF。Wire2s有界，defaultcleanup6s；forwardinghandles为private
synthetic active-state，不宣称actualdevice/start/forwarding效果。原assertions与public旧
consumer字节保持，只有三privatefixture增sharedstate；protocolstrings/parsers/default10s/
4MiBcap/public四variants及nativegetter/CLI/deps/refs保持，无新增reader task/queue。

准备阶段无livebuild时 `cargo clean -p ios-cli --profile dev` terminal0，删除2193构建
缓存文件/8.2GiB，非源码。首次15-testnarrow PASS后补两fixtures至17；未出现Rust失败。
第11节 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，session76233 terminal0，
validation/full PASS：2274Rust/13Python、0FAIL/two existing ignored，CLI734/Core1382/
public48。Rust/Python拆分；workspacefmt/check/strictClippy/MSRV、66features/fourgroups、
7+7docs/3snips、CABI、3.12wheel/sdist/install、56tooling/118help/38raw/5host/39CLI/
3backup合同均PASS。Frozen source SHA256
`0bbeca9d7402da4d2f1b900df8479c00ac3d8678a6e7fef2d7946f9978fb3287`；ios debug SHA256
`2773252713e3e238a9032dadde71e4d63cc7da6d24db5420a228a9c6f8f54165`。
Source冻结至full/commit，full后free3.8GiB，无storagefailure；日志/audits
`/tmp/ios-l01-companion-`，真实结果持久化本节/专题。

八文档收尾审计PASS：七Markdown/312 local links，testedcommit/frozen source/binary/
full/narrow/preparation/只读refs一致；Capability118源目录及actualCLIhelp PASS，正文/
maturity/既有evidence保持。全部31任务/subtasks/historyprefix精确保持，仅L01overview
行和末尾交付追加；文档期间产品source未改。

全部31任务/subtasks/history保持，总览9完成/11未开始/11进行中或待验收/条件不变。
L01/class4/overallgoal active；下一CompanionCLI setup/total/signal/output/primarycleanup/
Dropdisarm，后续WebInspector framing/client/correlatedsessions/boundedcaches，其他L01
协议/rawborrowedsessions/multi-requesttotals/longstreams/manualpairing/blockingresources、
L02–L04/Q04及第五至九类。Capability118正文/maturity/history保持仅追加hostevidence；
22device NOT_RUN，Chromium/installed3.9/3.13未重跑。Cooperativetimers不preempt同步
encode/decode/已运行worker；无Watch/iPhone/OSsignal/startforward/remoteStop/performance/
otherplatform验收。平台/设备/凭据/协议条件继续；无reference编辑/subagents/push/
publication/historyrewrite。

### 大类 4：L01 Companion CLI discovery/total/signal/cleanup ownership 交付（2026-10-05）

实现与受测提交：`c2fa6fb60c9bdd2f56514b7b4844fe39f596bc3d`，基于
`968dabbacf9d45e024c87cf8670791d9374dd634`。九份source/test/verifier文件，
1371 insertions/239 deletions，Linux x86_64/Rust1.97.1/MSRV1.80.1/项目Python3.12.13。
专题：[Companion CLI 生命周期与迁移](validation/companion-cli-control-2026-10-05.md)。

Finite Listen/Forward/Stop zero/MAX在显式或默认设备选择前拒绝InvalidInput；
clap/help/flags/defaults、classic/RSD routes及输出serde/newlines保持。Setup上限30s，
有限total从default discovery前开始，覆盖connect/start/wait/output且不重启。
同一pinned Ctrl+C覆盖全部阶段；setup及有限request-stage stop为typed cancellation，
开始后的Listen/Forward operator stop保持graceful。List/Get保留core10s请求；
无total监听使用unbounded next_event，不把request诊断期限当作idle timer。

Ready signal/due nominal deadline优先；每次operation poll先检查nominal deadline，
定时器注册尚未ready的间隙也不运行ready work，最终由原生timer提供实际Elapsed。
Finite expiry仍返回失败，旧synthetic TimedOut迁移为native Elapsed。Listener在完整
event/output后yield，ready流不饿死control；无新增reader/task/queue。Cooperative
timer不preempt同步codec/render/output/已运行worker。

Additive ForwardingHandle close/close_with_timeout消费handle，在创建future时立即
disarm旧Drop fallback，未poll/cancel也不晚发Stop。CLI在start后、output/wait前创建
该future，primary返回或graceful stop后独立await最多5s Stop，即使total已到期；
client/device持有到cleanup结束。Invalid session跳过unsafe cleanup，不伪造二次失败。
成功后cleanup失败返回cleanup；双失败私有aggregate保留两个original，source/category
沿primary，Display静态/Debug仅类别。Cleanup UnexpectedEof保留native object和既有Io，
business失败Protocol，issued cleanup expiry独立native duration/Elapsed。旧borrowed
stop/stop_with_timeout/Drop行为保留，可在business-failed explicit stop后继续旧fallback；
新consumed close无detached fallback。无remote stop/rollback/retry/ACK/join保证或
猜测StopListening。Zero保持core source-free Timeout；MAX安全clamp5s。

23个新Rust tests：CLI18/Core4/public1。最终narrow80528 terminal0为40PASS
(18/20/2)，minimum companion22(Core20/public2)。Pipeline26213 terminal0：Core
Companion36加public2、CLI Companion26(25command+既有main clap)、minimum CLI
alltargets check、实际匹配original stack5、严格Core/CLI Clippy及fmt通过。
Owned wire2s bound、SHORT20ms、actual5s cleanup用6s bound。200ms包含125mssetup及
125ms后续stage；是helper total证据，不冒称物理route。实际write_line_to注入writer，
不是Companion OS stdout故障验收。覆盖ready stop/expiry、pinned signal、partiallistener、
exact Start/Stop、primary output/native/timer与business/EOF/actual5s cleanup双失败、
setup/start/wait/committedcleanup wholecallerDrop/no follow-up、公开Send/unpolled close。

Verifier新增3个positive/contact/private-diagnostic controls，test_compatibility14PASS。
实际CLI39→55PASS，旧39保持；新16为12JSON Listen/Forward/Stop zero/MAX explicit/default、
2human admission、empty default及actual Linux SIGINT。14admission no endpoint contact；
empty只一次ListDevices/no extra connection；SIGINT仅owned fake ListDevices，
cancelled/companion/exit1与peerEOF，无设备service/pairing/Apple操作。

Preparation：首narrow50503 terminal101为测试bytearray条件长度不一致，改slice；
次73066 compiled为CLI13PASS/2FAIL，不能计Core/public通过。实际nominaldeadline/ready
work优先级问题已修复；另一项为UnexpectedEof误写Disconnected，改既有Io并检查native
kind。之后补3个CLI case，最终18全通过。每次源码编辑前进程已退出；无old-source
baseline-wide negativecontrol。无live build时focused cargo clean -p ios-cli --profile dev
清1917 cachefiles/7.3GiB，非source/reference清理；无storage failure。约3.7GiB清前、
8.7GiB清后、6.2GiB full前。

Full62300 terminal0，第11节Rust/Python拆分全部PASS：2297Rust/13Python，
CLI752/Core1386/public49，0FAIL/2既有ignored。Fmt/check/strictClippy/MSRV、66features/
4runtimegroups、7+7doctests/3snippets、CABI、3.12wheel/sdist/install、59tooling/
118actualhelp/38raw/5host/55CLI/3backup通过。Frozen source SHA256
`2363413f3539ee2a33f627aafe383fc00f7890d8ae87669aac73e7dace23e4ae`，
最终ios binary SHA256
`a3910bf0bf0656c18463b8904b98edbb29c0154fcaf4bb51e197c3ad54c1126c`。
九文件patch从freeze到full不变，并精确匹配实际实现commit；旧core assertions/privateflag
初始化以外、lifecycle helpers、borrowedstop/parsers/errors/default10s/4MiB/publicconsumer
保留；CLI clap/help/connect/route与5个旧tests保持，2个timing tests明确迁移zero/source。
无关compatibility helpers、reference HEAD/status保持。Logs/audits为/tmp/ios-l01-companion-cli-。

八文档收尾audit PASS：7Markdown/319local links、source/actualCLIhelp inventory PASS，
精确commit/frozen source/binary/full/narrow/preparation及readonlyreferences匹配。全部
31tasks/subtasks/旧history prefix精确保持，仅L01overview行和末尾交付追加；Capability118
正文/maturity/既有evidence保持，只增一项hostevidence。文档期间产品source未改。

全部31任务/subtasks/history保持，总览9完成/11未开始/11进行中或待验收/条件不变。
L01/class4/overallgoal active；下一WebInspector frame/client/correlatedsessions/bounded
caches，其他L01协议/rawborrowedsessions/multi-requesttotals/longstreams/manualpairing/
blockingresources，L02–L04/Q04及第五至九类。Capability118正文/maturity/history保持仅增
hostevidence；22device NOT_RUN，Chromium/installed3.9/3.13未重跑。平台/设备/凭据/协议
条件继续，无reference编辑/subagents/push/publication/historyrewrite；host acceptance不
完成L01、第四类或总体goal。

### 大类 4：L01 WebInspector client frame/start/event ownership 交付（2026-10-05）

实现与受测提交：`30fb63fe5d6a6530bb96270adbb59c27c96d1e27`，基于
`09d5fcd9eacc9f15dcda1a3b3bfc62e7d4fbd228`。四份source/test文件，1210 insertions/
55 deletions，Linux x86_64/Rust1.97.1/MSRV1.80.1/项目Python3.12.13。专题：
[WebInspector client 生命周期与迁移](validation/webinspector-client-lifecycle-2026-10-05.md)。

Additive is_usable/set_operation_timeout(OptionDuration)，默认None保持unbounded
client send/event；startup/timed event使用explicit/configured较短期限。发送在argument
clone/JSONencode前admit，共享XML/send/flush；event涵盖decode/routing/自动listing写入。
Due nominaldeadline在每次operation poll前阻止ready work，native timer给实际Elapsed，
无synthesizedcause/task/queue，codec/运行worker仍cooperative。

Zero在cache/IO前为source-free legacyTimeout；不可表示effectivebudget为Io InvalidInput，
explicit较短期限可clampMAX配置。Startup overflow从旧Protocol迁移；timed/readycache及
discovery zero迁移为checked admission；startup getter现报告effective operationbudget，
替代变化的reply remainder。Six variants/native wrapper/getter/constructors/rootpaths、
模型/serde/protocolstrings/16MiB raw framing保持，无bypass/deprecation alias；发布需审查
这些pre1.0 source/timing/reuse变化。

Direct frameguard跟踪accepted buffered/wire read/write、nativefailure/EOF/flush。
Partialcancel/expiry或codec失败失效，丢弃owned applications/pages/pending/automation
metadata；后续guarded wire/event-cache operation在IO前拒绝，invalid restore丢弃输入，
setterNone不复活。Empty idle/send cancel、unpolled及preIO XMLfailure可复用。Opaque
transport/buffers保持到Drop，connectionid保留；不是liveness/retry/RSS/remotecleanup证据。

Decoded event直到routing/automaticlisting完成才结束owner；其后emptyblockedwrite
cancel/expiry也失效，防止消费完incomingframe却留下半更新cache。Complete routing/
JSON/PID/shape错误保留original并失效；返回event之后的既有correlatedbusinessparser未改。
Startup sent report到CurrentState为logicalowner：cancel/expiry即使emptyread仍失效，
late complete state不能再消费。Legacy blocked report outertimer保持nativeTimeout，旧
reporttransport与responsefailure Disabled mapping保留；preserving API保留original。
Disabled不证明设备设置。无 guessed ACK/teardown/retry。

Open_application_pages只有empty/stillusable idle视为snapshot完成；partial或routing
失败返回原错误，配置total覆盖applicationrequest/reads并独立于idle，expiry仍错误。
DefaultNone保持旧idleprofile，rawhelpers caller责任与源码未改；Drop不做remote stop。

新16internal/1external tests。首compile43968 terminal101是23个test-only E0283 type
推断诊断，显式key/value types修正；该次没有任何testPASS。MAX admission fixture调整
输入role验证effectivebudget并加explicitshorterclamp，该执行前修正不冒称runtime failure。
次narrow23094为17PASS。补nativewrite/cancelledautomaticwrite后pipeline63338 terminal0：
finalfull17/minimum17，Core55+public3+integration1=59/CLI40/minCLIalltargets/
actualoriginalstack5/strictCoreCLIClippy/fmt通过。补JSON/PID nativebranches仅改新fixture
文件，product/public SHA与pipeline一致；supplement29765 terminal0再次17/min17/
strictClippy/fmt，再冻结四文件。

Owned2s外界、SHORT20ms及125+125ms/200ms totals。覆盖12entries/cacheadmission、nominal
now timer、emptyvsheader/body/buffered progress、unpolled/empty/partial writes、flush、
nativeEOF/BrokenPipe/PermissionDenied/plist/nondict/oversize/shape/JSON/PID sources、
committed startup/late completestate、send+readtotal、decoded event emptylistingwait/
cancel/expiry、read+automaticsendtotal、quiescencevstotal与original成功listing/cache、
公开13Send futures/6exhaustive variants。无physicalservice/browser/OSsignal/throughput/
RSS/oldsource baselinewide negativecontrol。每次sourceedit前进程已terminal；focused
cargo clean -p ios-cli --profile dev清1657cachefiles/6.6GiB，无livebuild/source/reference
清理/storagefailure。约2.9GiB清前、8.0GiB narrow后、5.6GiB full前。

Full23783 terminal0，第11节Rust/Python拆分全部PASS：2314Rust/13Python，
CLI752/Core1402/public50，0FAIL/2既有ignored。Fmt/check/strictClippy/MSRV、66features/
4runtimegroups、7+7doctests/3snippets、CABI、3.12wheel/sdist/install、59tooling/
118actualhelp/38raw/5host/55CLI/3backup通过。Frozen source SHA256
`3db99be39a1149eab1064b21ac22630b6f0610a2dca23d738deabbb5e5b0570b`，
最终ios binary SHA256
`503a27aa69f3cdff1637a4ccaaa0b08742798ad02a4c9ac29f2c8660635a52bf`。
Four-file patch从freeze到full不变并精确匹配实际implementationcommit。旧client全部
tests/assertions/startmapping/incomingrouting、model/wire/timing/Inspector/Automation/
protocolstrings/defaultNone/16MiB/errorfamilies/publicconsumer bytes保留，frontends/
dependencies/referenceHEAD/status未改。Logs/audits为/tmp/ios-l01-webinspector-。

八文档收尾audit PASS：7Markdown/324local links，source/actualCLIhelp inventory PASS；
精确implementation/frozen source/binary/full/narrow/compile/supplement/cachecleanup与
readonlyreferences匹配。所有31tasks/subtasks/旧history prefix保留，仅L01overview行与
末尾交付追加；Capability118正文/maturity/既有evidence保持，只增hostevidence。
文档期间产品source未改。

全部31任务/subtasks/history保持，总览9完成/11未开始/11进行中或待验收/条件不变。
L01/class4/overallgoal active；下一Inspector/Automation correlated totals/attach/late
reply ownership、bounded pending/skipped/application/page state，然后WebInspector
frontend/server lifecycle；其他L01协议/rawborrowedsessions/multi-requesttotals/
longstreams/manualpairing/blockingresources、L02–L04/Q04及第五至九类继续。Capability118
正文/maturity/history保持仅增hostevidence；22device NOT_RUN，Chromium/installed3.9/
3.13未重跑。平台/设备/凭据/协议条件继续；无reference编辑/subagents/push/publication/
historyrewrite，host profile不完成L01、第四类或总体goal。

### 大类 4：L01 WebInspector correlated session / retained event ownership（2026-10-05）

| 字段 | 交付记录 |
| --- | --- |
| 任务与范围 | L01；承接 client profile，覆盖 Inspector/Automation correlated command/attach、committed late reply isolation、pending/skipped count+logical bytes、u64 counter exhaustion、nested deadline inheritance；不完成 L01/第四类/31任务总体目标 |
| 实际实现提交 | `8011749571e2ae2a70ca8728984d0d93b56651bc`，基于 `4b8256684fbc15a125524f5a96d2616c0f5f3cbd`；12个source/test文件，独立本地实现提交，实际patch与受测freeze精确一致 |
| 实际行为 | send/flush/unrelated events/matching reply共用一个effective absolute deadline；complete command/setup/request后的abandoned owner失效client/drop owned metadata/events，其他session/clone不能消费late complete reply；原matched business error/reuse与event order保留；LOCAL pending+skipped1024/16MiB compact JSON caps；full u64 MAX一次后拒绝exhaustion，不wrap/reuse；finite scope继承到nested sends/startup/timed socket/events/automatic listings，退出恢复旧policy |
| 精确专题合同 | [验收与迁移](validation/webinspector-session-lifecycle-2026-10-05.md)、[生命周期](lifecycle-contract.md)、[公开接入](public-contract.md)、[错误](error-contract.md)、CHANGELOG/quality/Q01单项host evidence；所有主任务和子任务保留 |
| 主机验证 | Linux x86_64/Rust1.97.1/MSRV1.80.1/Python3.12.13；full70542 terminal0，第11节Rust/Python拆分全部PASS：2340Rust/13Python，CLI752/Core1427/public51，0FAIL/2既有ignored。Final narrow/minimum26；Core80+public4+integration1=85/CLI40/minCLIalltargets/actualoriginalstack5/strictCoreCLIClippy/fmt PASS；tooling59/actualhelp118/raw38/host5/CLI55/backup3与全部制品/文档/C检查PASS |
| 真实剩余条件 | Application/page缓存count+logical bytes/替换与disconnect释放仍待；WebInspector CLI/CDP/WebDriver/server setup/whole-flow total/pinned signal/join/output/primary cleanup继续；其他L01协议/raw borrowed/multi-request/long streams/manual pairing/blocking资源、L02–L04/Q04及第五至九类继续。22device NOT_RUN；Chromium/installed3.9/3.13未重跑；host不替代platform/device/credentials/protocol证据 |

Inspector send_command_and_wait与Automation command helper不再发送后重启timeout。
Inspector attach/unpaused包括setup/optional target；Automation attach包括request、两次
listing、socket setup和final connection-ID，page_id仅full attach成功后赋值。
Optional client cap与explicit budget取较短值，整个correlated wait用原absolute deadline。

Logical guard在complete outgoing command/setup/request后commit，cancel/expiry/错误即使
最后emptyread、前面unrelated frame已完成仍失效client并drop owned metadata/pending。
其他session/clone在该client上reject beforeIO，setterNone不能revive。Partial send/flush/
read继续由原frameguard持有，native/JSON/Protocol primary保留；unpolled/empty blocked
write/preIO admission可保留。无reader task/detachedcleanup/guessed ACK/reattach/retry。

Matched business response先完成wire obligation，再维持原业务处理：Inspector返回
error response object，Automation保留原Protocol message/reuse。Complete owner将skipped
events按原顺序restorefront；incomplete/drop将skipped与invalidclient一起丢弃，避免
消费late reply；restorefailure不替换已有primary。Matching、transportACK discard规则和
业务parser未改。Send-only attach(false)/command/bridge仍只承诺localwritecompletion；
session getters不是remote liveness，client失效后须new client+new session，不认证cached
target/page ID跨transport recovery。多command workflow整体预算留给后续owner。

Pending+skipped共享LOCAL1024events/16MiB compact JSON预算，已有pending计入combined
count+bytes，tags/keys/escaping/values由counting writer计数且不复制encoded payload。
Append/restore前检查；cached pop释放byteaccount，committeddrop清owned queuecapacity。
独立于旧16MiB wirecap；不是RSS/allocator/peakdecode memory/throughput/latency测量。
Application/page cache仍待本地有界化，不将该slice冒称all retainedclientstate有界。
Private WebInspectorLimit经feature-gated typed rule在既有Io下分类ResourceLimit，未加
publicvariant/stringclassification/借用无关decodererror。Command/transport u64 MAX可用
一次，None表示exhaustion；Inspector两counter先admit后consume，无panic/wrap/reuse。
Wirefixture证明原localu64域，不证明device支持MAX。

Scoped inherited deadline覆盖correlatedhelpers、startup、timedsocket、directtimedevent及
automaticlisting实际write，finitecap admission后nestedcall不再独立检查MAX造成overflow。
成功/错误/cancelscope退出恢复先前deadlinepolicy，独立effectiveMAX仍reject。Zero在
readyinput/IO前返回sourcefreeTimeout；effectiveoverflow由旧internalProtocol迁移到
IoInvalidInput，实际expiry保留originaleffective total/真实Elapsed，不restart/remainder/
fabricate。Nominaldue抑制readywork，codec/counting保持cooperative。Oldtiminghelper
assertions作为test-only bytes保持；6variants/constructors/signatures/rootpaths/defaultNone/
model/serde/wire/selectors/startDisabledmapping/businessparser/frontends/deps/reference
HEAD/status保持。这些pre1.0 timing/reuse/resource迁移已登记，不加unsafebypass/alias。

新25internal/1external tests。Actual Inspector/Automation125ms firstwritepoll block+
125ms late replyunder200ms；committed callcancel/expiryafterunrelatedframe/emptyread、
latecomplete和其他session/clone拒用；businessoriginal/order/nextID2；paused/unpaused
setupcancel/late target、targetshapeProtocol与Automation request/listingcancel/page未赋值；
nestedTarget malformedJSON originalsource；两session实际1025wireevents超限；combined
pending/skippedcount、exact16MiB/escaping/Unicode/cachedpop/re-admission/restore/drop；
zero/MAXreadyinput、unpolled/emptywritecancel/due真实timer；actual nested command/
startup/idle/automaticlisting MAXfinitecaps；actualdirect/Target/Automation fullu64 IDs/
exhaustion/transportfailure不consumecommandid；externalSend/6exhaustivevariants/native
timeout/clone/queuedlate/noIO。3s outerbound、SHORT20ms，无baselinewideoldsource
negativecontrol/device/browser/OSsignal/benchmark/otherplatform claim。

First narrow54036 terminal0为21PASS。补nestedcapfixture后18677 terminal101是2个
test-only E0283 dictionaryvalue类型诊断，该次无test执行；明确plistValue后72903为
25PASS，firstpipeline87754为minimum25/Core84/CLI40/stack5/strictgates PASS。补direct
eventinheritance及actualautomaticlistingfixture后product有变，95858为26PASS，pipeline
21871 terminal0为minimum26/Core85/CLI40/minCLI/stack5/strictClippy/fmt PASS。最后仅
newfixture file将125msdelay起点改为firstactualwritepoll，防止setup/scheduler提前
消耗阻塞阶段；product/public SHA与21871一致。Supplement57845 terminal0再次
narrow26/minimum26/strictClippy/fmt PASS，再冻结source。Sourceedit/追加build-test-check
pipeline前已有build/test进程均terminal；独立readonlyaudit不编辑source。

No livebuild时focused cargo clean -p ios-cli --profile dev首次清1657files/6.6GiB，约
1.1GiB free清前；full前再清778files/3.0GiB，约3.2GiB before/6.1GiB after；无source/
reference删除或storagefailure，full结束约1.9GiB free。第11节统一入口
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`
执行full70542 terminal0：2340Rust/13Python，CLI752/Core1427/public51，0FAIL/2既有
ignored，fmt/check/strictClippy/MSRV、66features/4runtimegroups、7+7doctests/3snippets、
CABI、3.12wheel/sdist build/install/import、59tooling/118actualhelp/38raw/5host/55CLI/
3backup全部PASS。Frozen12-file source SHA256
`93ce28b9c0926b4dc0b81991ca77f0f85e9495a17d89118957b823246cf3b8c2`，finaliosbinary
SHA256 `ec4f0d0e9b0eab456dd5e34e549ac372e8a41d90e5f0638c581f33a0d1cb534e`。
Actualimplementationpatch与freeze/full精确一致。Logs/audits为/tmp/ios-l01-webinspector-session-。

八文档audit PASS：7Markdown/330locallinks；source/actualCLIhelp inventory PASS。
All31tasks/subtasks/旧historyprefix保留，仅L01overview与末尾交付追加；Capability118
正文/maturity/既有evidence保持仅增hostentry。精确commit/source/binary/full/narrow/
compile2diagnostics/fixtureonlysupplement/cachecleanup和readonlyreferences匹配，文档
期间产品source未改。

所有31主任务/subtasks及历史prefix保持，总览9完成/11未开始/11进行中或待验收/条件
不变；L01/class4/overallgoal active。接着application/pagecache bounds，再frontend/
server lifecycle，然后其他L01、L02–L04/Q04及第五至九类。Capability118正文/maturity/
既有evidence不变，只追加hostevidence。22device NOT_RUN，Chromium/installed3.9/3.13
未重跑；platform/device/credentials/protocol条件继续。无subagents/referenceedit/push/
publication/historyrewrite，hostprofile不完成L01/第四类/总体goal。

### 大类 4：L01 WebInspector metadata / staging / snapshot admission（2026-10-05）

| 必填字段 | 实际交付与边界 |
| --- | --- |
| 任务/范围/状态 | L01/class4/all31 active；application/page/availability组合cache、staged map/listing Vec和flattened snapshot LOCAL count/logical-byte admission与释放主机子范围验收，不完成L01/本类/总体 |
| 受测实现 | `20d6fc5774e9f2bfdc243de194ee7e62f084967c`，基于 `602ef1d3780ab2e4a5f72aa62260ac2fa45c56d6`；明确5source/test files，1168+/11-；新16internal+1external，原client/public consumer及前两lifecycle/session断言byte-identical |
| 合同/迁移 | [专题验收](validation/webinspector-cache-lifecycle-2026-10-05.md)、[生命周期](lifecycle-contract.md)、[公开接入](public-contract.md)、[错误](error-contract.md)、CHANGELOG/quality/Q01单项host evidence；固定LOCAL限制为pre-1.0行为变化，需release review，不升级maturity |
| 主机验证 | Linux x86_64/Rust1.97.1/MSRV1.80.1/Python3.12.13；full45509 terminal0，第11节Rust/Python拆分全部PASS：2357Rust/13Python，CLI752/Core1443/public52，0FAIL/2既有ignored。Final narrow/minimum17；Core96+public5+integration1=102/CLI40/minCLIalltargets/actualoriginalstack5/strictCoreCLIClippy/fmt PASS；tooling59/actualhelp118/raw38/host5/CLI55/backup3及全部制品/文档/C检查PASS |
| 实际剩余 | WebInspector CLI/CDP/WebDriver/server setup/whole-flow total/pinned signal/join/output/primary cleanup继续；decoded plist object-reference expansion/recursion/allocation peak/CPU preflight与fuzz属L01/Q04未验收；其他协议/raw borrowed/multi-request/long streams/manual pairing/blocking资源、L02–L04/Q04及第五至九类继续。22device NOT_RUN；Chromium/installed3.9/3.13未重跑；host不替代platform/device/credentials/protocol条件 |

固定LOCAL caps为1024applications/rawappentries/pageapplicationsets（含empty/orphan）、
4096aggregatecachedpages/rawlistingentries。Availability+applications+application_pages
组合compactJSON上限16MiB，包含mapkeys/duplicatedmodelIDs/containers/escaping/UTF8；
stagedmaps与duplicate-preservinglistingVec各16MiB。额外flattenedApplicationPage
projection16MiB，在cacheadmission前约束同一大application被每page重复copy的放大。
这些不是Apple/device协议上限，不是RSS/allocator/peak/CPU/latency/throughput证明。

Rawcount在reserve/per-entryparse前拒绝，staging以checkedreplace/sub/add计数后才
insert/clone/append。Borrowedprojection计算替换后组合cache/snapshot，无完整旧snapshot
clone或encodedJSON副本。拒绝不partialmutate旧snapshot；successlistingevent保留原
duplicateorder，cachelastvalue/firstIDorder保持；connectedsnapshotabsentpageowner释放、
emptytableDrop/survivoroutershrink，page替换Drop旧map，disconnect释放两cachepayload/
budget并Dropemptycapacity。原自动listing每uniqueID一次保持。

CompleteincomingroutingResourceLimit通过既有eventowner失效/dropmetadata/pending，
discovery不吞成idle成功，之后guardedIO拒绝。PrivateWebInspectorLimit精确typed分类
仍在IoOther下，6publicvariants保持，无stringmatch/OutOfMemory/其他decoder替代；
resource diagnostics仅resource/limit，不带app/title/URL。Privatehandler测试只为查看
admission前无mutation，绕过frameowner的usable状态不是publicreuse合同。

实际测试包括1025跨frameapp增长、rawoversizedmalformedapp/listcount优先、1024empty/
orphanpageowners、4096/4097跨app和实际跨framepages、exact16MiB组合metadata与
Unknownavailability/keys/JSONescape/Unicode、替换缩小/re-admit/disconnectcapacity、
duplicateIDs/eventcacheorder/oneautomaticsend/absentowner释放、flattenedboundary或
1bytebelow及下一namechar拒绝、actual8MiBapp+2smallpages拒绝和stagedduplicateVec。
Actualbplist00共享8MiB字符串frames均assert<16MiB，分别拒绝stagedappmap/listingVec；
publicSend/exhaustive6variants/ResourceLimit/Io/clearedmetadata/后续readwrite无IO保持。
10souterbounds；无baseline-wideoldsourcecontrol/device/browser/OSsignal/benchmarkclaim。

验证轨迹如实保留：firstnarrow75451 terminal0为13PASS；加入实际binary/aggregate/public
fixtures后33329 terminal0为17PASS，本profile无compile/runtimefailure。Sequentialgates
全部exit0，minimum17/Core102/CLI40/minCLI/stack5/strict/fmt PASS。Full45509 terminal0
为2357Rust/13Python，workspacefmt/check/strictClippy/MSRV、66features/4runtimegroups、
7+7doctests/3snippets、CABI、3.12wheel/sdistbuild/install/import、tooling59/help118/raw38/
host5/CLI55/backup3全部PASS。22device NOT_RUN，Chromium/installed3.9/3.13未重跑。
所有已有build/test terminal后才edit或启动另一pipeline，观察超时不算进程退出。

FocusedCLIcacheclean仅在allbuilds terminal时执行：initial1301files/5.1GiB（free1.9GiB
before/5.7GiBafter）；prefull446files/1.7GiB（3.5GiBbefore/5.3GiBafter）；full后约695MiB。
无source/reference删除或storagefailure。五文件freezeSHA256
`987d5836c804bdba0cde773311c3726d2b08af787ca9bb1f61c96de4ee5f52b7`；iosbinarySHA256
`75af15d7cb44043e82dc630583077d90b38c61e9cba73a5e7a50563c559b6fcd`。
Actualcommitpatch与freeze/full精确一致，logs/audits使用/tmp/ios-l01-webinspector-cache-。

Lockedplist1.7.1binarywriter共有scalarreferences；这些合法smallframes可在decoder展开，
raw16MiB检查先于plist::Value解析，model/cacheadmission在解码之后。故decoderpeak/
recursion/object-referenceexpansion/CPU仍待证据和preflight/fuzz，计数/codeccooperative；
不能声称全部allocation有界。Wire/model/selector/rawhelper/mapping/defaultNone/public
signatures/rootpaths/businessmatching/frontend/dependencies和referenceHEAD/status不变。

全部31主任务/subtasks/历史prefix保持，仅当前L01overview与末尾交付追加；9完成/
11未开始/11进行中或待验收条件不变，118capability正文/maturity/prior evidence保持仅
加hostentry。接着frontend/serverowner，再其他L01、L02–L04/Q04和第五至九类；禁止
subagents/referenceedit/push/publication/historyrewrite，host不完成本类或总体goal。

八文档audit PASS：7Markdown/340locallinks与source/actualCLIhelp118 inventory；
all31task/subtask/historyprefix和118capability正文/maturity/prior evidence保持，
仅L01overview/末尾交付/单项hostentry追加。Exactcommitted/frozensource/binary/
full/narrow/gates/cachecleanprovenance匹配，文档期间productsource未改，reference
HEAD/status不变。L01/class4/overallgoal active，继续独立可执行frontend工作。

### 大类 4：L01 WebInspector CLI discovery primary error retention（2026-10-05）

| 必填字段 | 实际交付与边界 |
| --- | --- |
| 任务/范围/状态 | L01/class4/all31 active；CLI application/page discovery失效Timeout保留主错误的主机子范围，不完成setup/whole-flow/signal/join/cleanup/本类/总体 |
| 实现 | `db7960ae4d8643c1c8849e4a8795a21cdc821ee6`，基于 `e1befa3e25fa0b0c2b4332c8e69698dbde2cbfc9`；明确2source/test files，384+/2-；7newinternal各两workflow，原workflow全文除2guard/testmodule声明外byte-identical，旧tests/selector/deadline/helpers/output/core/protocol/dependency保持 |
| 合同/迁移 | [专题验收](validation/webinspector-discovery-lifecycle-2026-10-05.md)、[生命周期](lifecycle-contract.md)、[公开接入](public-contract.md)、[错误](error-contract.md)、CHANGELOG/quality/Q01单项host evidence；原先被后续Protocol覆盖的native Timeout恢复，pre-1.0需release review，不升级maturity |
| 主机验证 | Linux x86_64/Rust1.97.1/MSRV1.80.1/Python3.12.13；full92892 terminal0，第11节拆分全部PASS：2364Rust/13Python，CLI759/Core1443/public52，0FAIL/2既有ignored。Corrected narrow/minCLI7，CLI47/Core102/coreonlywebinspectorcachemin17/minCLIalltargets/originalstack5/strictCoreCLI/fmt PASS；tooling59/help118/raw38/host5/CLI55/backup3及制品/文档/C PASS |
| 真实剩余 | WebInspector其他frontend/server setup/whole-flow/cached due admission/pinned signal/join/output/independent primary cleanup，js-shell/CDP/WebDriver资源和decoded plist expansion/depth/peak/CPU/fuzz继续；其他L01、L02–L04/Q04及第五至九类继续。22device NOT_RUN，Chromium/installed3.9/3.13未重跑；host不替代platform/device/credentials/protocol条件 |

CLI wait_for_application/wait_for_inspectable_page原将所有issuedTimeout作为idle，
partialheader/body/automaticlistingwrite导致clientinvalid后，下一applicationlistrequest
用staticunusableProtocol覆盖主nativeTimeout。现在仅clientis_usable时retryidle；
失效直接保留原Elapsed/duration/Timeoutclass。Emptyusableidle仍retry，cachematch/
成功selection/pagekeyprecedence/helpers/500msidlecap/CLIgrammar/output保持；无新
API/errorvariant/reader/ACK/remotecleanup/protocolguess。此profile不证明wholeflowowner。

7actualServiceStream/ownedduplexfixtures各覆盖两workflow：partialheader/body后
queuedlatecompleteframe/无retrywrites；64byteduplex真实automaticlistingblocking及
partialoutgoingframe；观测第二actualrequest证明emptyidle后真实app/pageselection，
explicitkey优先不匹配bundle、cachedmatch无write；原EOFUnexpectedEof/plist::Error/
1025rawappcountResourceLimit保持。20msconfiguredidle/3souterbound，无伪造deviceguard，
无device/browser/OSsignal/baseline-wideoldsourcecontrol/benchmark/RSS/otherplatformclaim。

First84258 terminal101：23newtestonlyE0283dictionarykey推断diagnostics，NO TEST
EXECUTION/PASS；显式Stringkeys修正后60870 terminal0/7PASS。Pipeline42910全部exit0，
minCLI7/CLI47/Core96+public5+integration1=102/coreonlymin17/minCLIalltargets/stack5/
strictCoreCLI/fmt PASS。CLIdependency仍enablecorefull，core-onlywebinspector另检。
Full92892 terminal0：2364Rust/13Python；workspacefmt/check/strictClippy/MSRV、66features/
4runtimegroups、7+7doctests/3snippets、CABI、3.12wheel/sdistbuild/install/import、tooling59/
help118/raw38/host5/CLI55/backup3全部PASS。原2ignored和22device NOT_RUN保持。

Allbuilds terminal后才edit或启动新pipeline，观察超时不算退出。FocusedCLIcacheclean
1301files/5.1GiB（free695MiBbefore/4.5GiBafter）；全部gates terminal且2.8GiBfree后
Coredevcacheclean48515 exit0，65236files/72.1GiB，free71GiB；full重建相关targets后
约49GiBfree，无source/reference删除或storagefailure。TwofilefreezeSHA256
`810c27f44d0b5f192dd77904694bc4f16926ab00b2c11c74d21db3ad1bbd19d2`；iosbinarySHA256
`2455e4279eb5d7313278e12c9ff827487f5d5ee9330e347dd7bdd7e255d53a85`。
Actualcommit与freeze/fullpatch精确一致；logs/audits使用/tmp/ios-l01-webinspector-discovery-。

Outputmacros原已传播IO，后续重点是same-total/primarycleanupscope与injectedwriter证据，
不把现有输出描述成忽略失败。其他cached-due/signal/wholeflow/independentcleanup/
joinedtask/CDP/js-shellcontinueinvalid等仍待。Readonly另核L02生产TunnelAgent全局
lifecyclelock跨ensure/refreshnetwork，Bquery/stop隔离待实际fake测量；不是性能PASS。
Plistreader已有cyclecheck，仍需expansion/depth/CPU与boundeddecode/fuzz，非peak/RSS证明。

全部31主任务/subtasks/历史prefix保持，只L01overview/末尾交付/单项hostentry追加，
9完成/11未开始/11进行中或待验收条件不变，118capability正文/maturity/prior evidence
保持。L01/class4/goal active，接着其他frontendowner，再其他L01、L02–L04/Q04和
第五至九类；无subagent/referenceedit/push/publication/historyrewrite或device操作。

八文档audit PASS：7Markdown/348locallinks与source/actualhelp118 inventory；all31
task/subtask/historyprefix、118capability正文/maturity/prior evidence保持。Exact
source/binary/full/narrow/23compile diagnostics/gates/cacheclean provenance匹配，
文档期间sourcefrozen，referenceHEAD/status不变。L01/class4/overallgoal active。

### 大类 4：L01 finite WebInspector CLI command ownership（2026-10-05）

| 字段 | 当前真实交付 |
| --- | --- |
| 实现 | `fa3d3f7ec523bad235c2d3986e28bf229998d1c2`，基于 `418b51dbe199e1a1dd1d9a09f4554ffbda065db5`；明确11source/test files，2147+/208-；21newRust/30newactualCLI/4newtooling |
| 范围 | opened-tabs/eval/launch默认发现、建连、startup、discovery、attach、每个命令及output共用checked absolute total/同一pinned Ctrl+C；每阶段配置client cap不延长total；fully attached usable session另有5scleanup，primary与secondary及automation两步cleanup错误都保留 |
| 合同 | [专题验收](validation/webinspector-command-lifecycle-2026-10-05.md)、lifecycle/public/error/CHANGELOG/quality/Q01host evidence；zero从100msfallback改为pre-discovery source-free Timeout；500msidle在total内；cleanup失败可在已output后exit1，pre-1.0需review |
| 当前验证 | fresh pipeline55428 authoritatively terminal0，完整第11节拆分全部PASS：2385Rust/13Python，CLI780/Core1443/public52，0FAIL/2既有ignored；narrow/min21，CLI68/Core102/coreonlyminimum17/minCLIalltargets/originalstack5/strictCoreCLI/fmt/targetedtooling18/actualCLI85全部PASS |
| 保留条件 | 22device NOT_RUN；Chromium/installed3.9/3.13未重跑；无browser/other-platform/device/remoteACK/retry/RSS/decoderpeak/baseline-wide old-source control；interactive/server/join与其他L01、L02–L04/Q04及第五至九类继续 |

root以前在finite命令timer/signal之前执行unbounded默认发现，本轮移动到command owner。
Firstdefault（包括Network先列出）和explicit bypass保持；empty诊断保持。一个原有
root predicate assertion按必要行为迁移，原task/shell testmodules byte-identical；
不能把这轮描述成全部旧assertions都未改。Core API/6variants/models/wire/dependencies、
CDP/Seleniumexplicit selector/security及interactive现有路径不改。

Ready signal优先于due timer/work，failed signal保留IO；due total抑制ready work。
单poll跨期限后返回成功也必须由native timer报Elapsed，已返回native失败保持primary。
Output/serialization依旧cooperative，无强制CPU抢占/benchmark承诺。
Tabs没有attached session；Eval精确session socket teardown；Launch先Stopownedcontext
再teardown forwarding socket，两步共享独立cleanup total，不再用caller remainder。
Incomplete/invalid client不发猜测恢复请求，stream/device guard保留到cleanup结束再Drop。
这只证明local accepted wire/flush/ownership，不证明remoteACK/rollback或可重试。

21tests覆盖7control以及14oneshot groups：三命令human/JSON真实帧、first/explicit selection、
default/nativeconnect/partialEOF/plist、跨connection/start总预算、committedevalcancel/expiry
无retry/teardown、matchedbusiness清理、实际writer BrokenPipe与双错、outputexpiry/cancel后
独立cleanup/delayedflush、completedoutput后cleanup失败、Stopbusinesserr后socketexpiry。
实际owned CLI85（新增30），zero/两种overflow在default/explicit前拒绝、empty、stalled
200ms ListDevices/EOF、实际Linuxchild SIGINT与human invalid，no realdevice/pairing supplied。
Tooling63（新增4）；full的66feature/4runtime、7+7doctests/3snippets、C、3.12wheel/sdist、
installedimport、help118/raw38/host5/backup3全部PASS；不升级capability maturity。

历史开发先19tests10PASS/9FAIL，再20tests18PASS/2FAIL，再20tests19PASS/1FAIL，暴露
fixture对省略pause key、完整eval封套及native sources/classes的错误假设；late-ready
control在driver修正前0PASS/1FAIL。历史full30439/71014及/tmp记录在环境续接后不可读取，
不用它们代替当前验收。保留源码后fresh55428重跑13gates至terminal0，当前证据写入
项目ignored target/validation/webinspector-command-2026-10-05/，无需cacheclean。

Freeze patch SHA256：
`a61bc9678b409a9264049a2d1db784fbc5ee52ada160487c215ffe1e19a0ce7f`；
binary SHA256：
`5169af3728eecdb8e281852b9ddaa517d00b6bfeba4f351b31a2f9e45e93e898`。
Actualcommit与frozen/tested11filepatch精确一致，referenceHEAD/status保持。
协议selector/字段使用existingCore API与只读reference；WebKitshared inspection/automation
target close map提供source reasoning，详情见专题链接，不以source推断替代真机。

全部31任务/subtasks/既有交付prefix及118capability正文/maturity/prior evidence保持，
仅L01overview/末尾交付/单项hostentry追加；9完成/11未开始/11进行中或待条件不变。
L01/class4/总体goal active，继续js-shell/CDP/WebDriver/server owner，再其他L01、
L02–L04/Q04与第五至九类。无subagent/push/publication/historyrewrite/referenceedit/device操作。

文档audit PASS：8documents/7Markdown/354locallinks与source/actualhelp118；all31
task/subtask/historyprefix及118capability正文/maturity/prior evidence保持。Exact
source/binary/freshfull13gates/current21tests匹配，文档期间无product edits，references
HEAD/status不变。L01/class4/overallgoal active。

### 大类 4：L01 interactive WebInspector shell ownership（2026-10-05，整类进行中）

实现与受测提交：`8ac37b7d52ce74dc29c6f11e06c82bcffba879fc`，基于
`26de9e25b93be089832e843fc012ab4766dbd842`；证据见
[WebInspector shell lifecycle](validation/webinspector-shell-lifecycle-2026-10-05.md)。
17 个变更文件（18 个 frozen paths，包含 source rename），2751 insertions/865 deletions。
L01/class4/全部31 tasks 保持 active；本记录是主机子 profile，非真机或整类出口。

| 字段 | 实际交付与边界 |
| --- | --- |
| 任务与范围 | L01 js-shell frontend：input/default/connect/start/Safari/discovery/attach/navigation/metadata 共用 checked setup total；之后 idle unbounded/cancellable，每条 expression/interpretation/output 新建 total；one pinned signal 贯穿全部阶段 |
| 实际变更 | ready blank input yield；每次协议阶段刷新 remaining client cap；continue-on-error 仅继续 complete JS/remote business response；native wire/deadline/codec/input/output 停止并保留 source，不复用 invalid client 或重试 late reply |
| 信号与清理 | setup Ctrl+C typed Cancelled/exit1；active successful Ctrl+C 为 normal stop，failed registration 保留 native IO；fully attached usable session 才执行 independent5s existing socket teardown；保留 primary/session cleanup/stdin restoration，正常 stop 也可能因 cleanup 退出1；stream/device guards 保持到 cleanup，未完成 attach/invalid client 不猜 recovery |
| Linux input | owned AsyncFd pipe/TTY，无 detached blocking read；dup shared status flags 明确恢复，Drop best-effort；preexisting nonblock 不变；regular file cooperative/dev-null EOF/unpollable dev-zero 保留 native registration failure；macOS/Windows 仍用旧 Tokio stdin，取消/退出实现及验收待条件 |
| 兼容与源码审查 | session/result/error compact JSON、human stdout/stderr/prompt/EOF/exit、full Runtime response、page-key/Safari/URL encoding 保留；original task/shell modules、14 finite assertion functions、control source/tests byte-identical；shared Session/cap/cleanup 仅移动/private visibility/fmt/trailing comma，无 logic change；core/bindings/public variants/wire/server access policy 不变；Cargo.lock 仅 existing libc CLI Linux edge，无版本/checksum 升级 |
| 历史失败 | first compile101/7 test temporary-borrow errors/no tests；second narrow23PASS/2FAIL（native EOF class 与 cleanup source traversal 的 fixture assumptions）；修正后 third31PASS。Pipeline74954 最小/finite/CLI/Core/core-min/minCLI/stack PASS 后 strict Clippy101（test-only JSON import）；cfg(test) 修正。不计为当前通过 |
| 审查后的真实验证 | preceding pipeline69263 all14/full2416Rust/13Python PASS；提交前发现 child-ready marker 位于 metadata publication，收紧到 actual first input poll，only interactive/tests.rs 改变，product bytes 保持；current reviewed pipeline44479 authoritatively terminal0，all14 gates 和 section11 full PASS |
| 当前窄检查 | shell31/min31、unchanged finite21、CLI99、Core96+public5+integration1=102、core-only webinspector minimum17、minimum CLI all-targets、original stack5、strict Core/CLI Clippy/fmt、targeted tooling22、binary/actualCLI95 PASS；CLI dependency full 与 core-only minimum 分开 |
| 完整统一入口 | 2416 Rust/13 Python，CLI811/Core1443/public52，0 FAIL/2 existing ignored；workspace fmt/check/strict/MSRV1.80.1、66 features/4 runtime、7+7 doctests/3 snippets、C ABI、3.12 wheel/sdist install/import、67 tooling/118 help/38 raw/5 host/95 actualCLI/3 backup PASS |
| 主机 fixtures | 23 shell +8 Linux input tests；owned plist stream/private Source/input/writer，fake Connection 无 forged ConnectedDevice；actual PTY/readiness/flags/registration error、stdin-only child/runtime exit；whole shell 在 first input poll 后接收真实 owned SIGINT、一次 cleanup、parent writer 仍打开时 child/runtime exit；10 new actual ios setup/default/zero/overflow/empty/stall/SIGINT cases，无设备服务或配对/Apple操作 |
| 真实剩余 | CDP/WebDriver/server setup/request totals、revocation/primary cleanup/joined child owners，其他 L01 协议和清理；L02–L04/Q04 的 manager isolation、重连、resource/decoder expansion/depth/peak/CPU/fuzz/baseline；第五至九类所有子任务继续；22 device NOT_RUN、Chromium/installed3.9/3.13 NOT_RERUN，无 browser/其他平台/远端ACK/safe retry/long-run/RSS/decoder peak 或 baseline-wide old-source acceptance |

当前唯一完整验收为 reviewed/ 下的 pipeline44479；root/ 和 final/ 的较早源码、
失败及通过均保留各自 snapshot，不覆盖，不作为 latest fixture 的替代验收。
Linux x86_64/Rust1.97.1/MSRV1.80.1/project Python3.12.13；统一入口：

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Durable ignored evidence：
target/validation/webinspector-shell-2026-10-05/reviewed/。
Source patch SHA256：
`9385b45600f00f84590743dab1a8abbdfc18a29cb286d88784e50a81cd9553d4`。
Actual ios debug binary SHA256：
`ebebc83ddc3cfe282d3a4447fa1269636b7a7aa3701e45db47bbcf194abe7d7c`。
Actual commit 与 frozen tested patch 精确一致；refs HEAD/status 不变，无 cache clean。
Timing/zero/continue-on-error/cancellation/cleanup exit migration 留待 pre-1.0 review；
host evidence 不替代平台、真机、凭据或协议条件，local write/flush 不等于 Apple ACK。

全部31任务及subtasks、原交付历史prefix、118capability正文/maturity/prior evidence
保持；9完成/11未开始/11进行中或待条件不变。仅L01 overview/末尾交付与单项host
evidence追加。实现先单独提交，再文档收尾；总体目标保持 active，接着推进
CDP/WebDriver/server owner、其他L01、L02–L04/Q04与第五至九类。无subagent/push/
publication/history rewrite/reference checkout edits/device操作。

文档 audit PASS：8 documents/7 Markdown/363 local links/source与actualhelp118；
all31 task/subtask/history prefix、118capability正文/maturity/prior evidence保持。
source/binary/full14gates/current31 tests匹配，文档期间无product edits，reference
HEAD/status不变。总体goal仍为active，继续后续独立工作。

### 大类 4：L01 CDP page terminal idle/native primary retention（2026-10-05）

实现：`2e22aaf7da2c47fff1654c557882e8cb9a121517`，基于
`4de5c00df11ca72f963f1814d643610b0ebe97f3`；明确3source/test files，476+/6-。
验收见 [CDP page lifecycle](validation/cdp-page-idle-lifecycle-2026-10-05.md)。
生产只改 page bridge 的 Timeout retry guard：仅 client.is_usable 时继续 idle。
Partial header/body 或 automatic listing write expiry 使连接失效时，保留第一份
native Elapsed/Timeout，不让下一轮 local Protocol 覆盖，不再写 retry/teardown。
Healthy empty idle 仍恢复并转发 nested Target reply，正常 WebSocket close 使用
既有 selector。五个 cfg(test) helper 仅扩大 private visibility，旧body/assertion
逐字保持；Core/public/bindings/dependencies/access/cleanup/browser代码不变。
Returned error correction 留待 pre-1.0 release review，不升级 capability maturity。

五个 actual owned Axum WebSocket/duplex plist/private InspectorSource tests：
partial header/body、真实 connected-app-list routing blocked write、multiple empty
idle 后 target relay/normal teardown，以及 actual EOF/plist/legacy oversize errors。
50ms idle/4s outer fixture bound 不作为 performance 承诺；stream Drop、peer EOF、
单次 connect/no further bytes 均实际观测，无 forged device 或 public fake backend。
Direct private handler observation 不替代 admission/revocation 或外部 CDP/device验收。

历史准确保留：23098 compile101/3 E0283 diagnostics，无test execution。
80012 terminal101/1PASS3FAIL，两个 terminal cases Protocol 替代 Timeout，第三项
误用 applicationUpdated 而 outer bound；修正事件后 before-product56035 terminal101/
1PASS3FAIL，三项均 actual Protocol vs expected Timeout，healthy idle PASS。
修复 guard 后7527 terminal101/4PASS1FAIL，新增 plist assertion 误认为 Protocol，
既有实际class为IO；只修该assertion，19929 terminal0/5PASS。

Frozen pipeline36134 authoritatively terminal0，全部15gates通过。新tests/allfeatures
与minCLI5/5，finite21/shell31/CLI104/Core96+public5+integration1=102/coreonlycachemin17/
minCLIalltargets/originalstack5/strictCoreCLI/fmt/targetedtooling22/actualCLI95 PASS。
第11节完整 Rust/Python 拆分：2421Rust/13Python，CLI816/Core1443/public52，0FAIL/
2既有ignored。Workspacefmt/check/strictClippy/MSRV1.80.1、66features/4groups、
7+7doctests/3snippets、CABI、3.12wheel/sdistbuild/install/import、tooling67/help118/
raw38/host5/CLI95/backup3全部PASS。22device NOT_RUN，Chromium/installed3.9/3.13未重跑。
所有构建终止后才改源码/启动下一验证；观察超时不等于退出。

Frozen source patch SHA256：
`9b45ef230001de695e42149a97f0a3eb217c517059b33b8664e2160a5689a42d`。
Actual ios debug binary SHA256：
`2fe7cc98689301125679d1776244f17e8412e8b9d5d449fd1910fd8f0df48031`。
Actualcommit 精确匹配 frozen/tested patch；证据在 ignored
`target/validation/cdp-page-idle-2026-10-05/`。Refs HEAD/status不变，无cacheclean。

L01/class4/overallgoal active。Page/browser/WebDriver/server setup/request/output
totals、独立primary cleanup、server cancellation、revocation/joined child/stored
tasks，其他协议/清理、L02–L04/Q04与第五至九类继续。Stored100ms/browser2s cleanup
policy 未改；无remoteACK/retry/rollback/other-platform/benchmark/RSS/decoderpeak验收。
全部31主任务及subtasks/history、118capability正文/maturity/prior evidence保持，
9完成/11未开始/11进行中或待条件不变。仅追加本profile，不结束总体目标。

文档audit PASS：8documents/7Markdown/367locallinks与actualhelp118；all31
task/subtask/history prefix及118capability正文/maturity/prior evidence保持。
真实commit/source/binary/15gates/full36134/five tests匹配，文档期间无product
edit，refsHEAD/status不变。实现与文档分开提交，接着server/其他独立工作。

### 大类 4：L01 CDP/WebDriver server setup totals（2026-10-05，整体进行中）

实现 `82db3a8699f99de4d7275baa896e408e543023ed`，基于
`3a5a1d9c1b6c5a697bc20f79a26db0ab08d30b9f`；明确9source/test files，781+/116-。
证据见 [WebInspector server setup](validation/webinspector-server-setup-2026-10-05.md)。
CDP full-page discovery、page/browser-page constructor 和 WebDriver runtime
constructor 各自从 lock/source 前到 startup/discovery/attach/context creation
共用 checked absolute total；每阶段 cap 更新为 remainder，成功后清除 setup cap，
保持 active 的既有政策。复用既有 frontend controller，无公开 API/协议猜测。

精确 CDP target 在 actual admitted match 后 attach，不再等全量 snapshot quiescence。
Missing target 在 setup total 内等待；只有 usable idle 可继续，invalid errors 原样返回。
全量 CDP list 与 WebDriver discovery 使用独立500ms quiet window，由 setup total
约束；原 server timeout 不再逐阶段重启，短预算可真实失败。CLI parser/listener
admission、startup/signal/output/publication、active command totals 仍待，不把私有
ServerState zero/overflow fixtures 描述成 actual CLI zero admission。
Browser incomplete attach 不再尝试猜测 teardown；stream/device/lock 随 owner Drop。
Fully attached/business-failed constructor 的 primary cleanup、取消后的异步收尾、
stored-session revocation 和 joined child ownership 仍待；100ms/2s cleanup policy未改。

15 actual owned fixtures（CDP9/WebDriver6）覆盖 Source200ms vs admitted50ms、
zero/clock overflow source-free、真实 blocked writer/page lock、600ms total 中的
200ms connect/500ms startup wait、connection/startup/discovery remainder，以及
caller cancel after actual attach/create frames 的 stream Drop/EOF/lock reacquire。
既有 Automation peer 验证 create/get/close，750ms setup 后 idle 和400ms active reply
证明 cap 不泄漏；650ms total 约束 committed context creation 的未回复等待。
Stop 既有只关闭 context，测试实际 teardown0，不伪造 forwarding/Apple cleanup。
Actual local WebSocket/private source 不替代新 admission 或 external-client/device验收。

Before-product69631 terminal101/0PASS4FAIL，四个 private source 均在预算后返回；
这证明 frontend seam 缺少 total guard，不声称 production DeviceSource 无单次cap。
42493 terminal0/4PASS，91234 terminal0/CLI108；33735 terminal0/14PASS，最后增加
browser attach caller-cancel，18610 terminal0/15PASS。无 compile/test failure after
product change。Atomic patch context failure 未应用；audit 首次仅因 rustfmt trailing
comma normalization 失败，核对实际 diff 后只修 ignored audit，source 未改。

Frozen pipeline42886 terminal0，全部16gates PASS：narrow/minCLI15/15、page5/finite21/
shell31/CLI119/Core96+public5+integration1=102/coreonlycachemin17/minCLIalltargets/
originalstack5/strictCoreCLI/fmt/targetedtooling22/actualCLI95。第11节完整拆分全部PASS：
2436Rust/13Python，CLI831/Core1443/public52，0FAIL/2既有ignored。Workspacefmt/check/
strictClippy/MSRV1.80.1、66features/4groups、7+7doctests/3snippets、CABI、3.12wheel/
sdistbuild/install/import、tooling67/help118/raw38/host5/CLI95/backup3全部PASS。
22device NOT_RUN，Chromium/installed3.9/3.13未重跑。所有build终止后才改源码或启动
新验证；观察超时不等于退出。无cacheclean。

所有原 CDP/WebDriver/task/assertion modules、五个 CDP lifecycle functions、finite/
shell/control tests 与 active cleanup/security 保持；controller 只改 private visibility/
新增 cap，原logic保持。Shared bridge fixture 100ms→1s 为500ms quiet stage留预算，
旧assertions不变；cfg(test) source/peer增加 delayed ownership/creation stall/context
delay 并复用既有 selectors，默认行为保持。Core/public/bindings/dependencies不变。
Timing/selection/missing-target/error cleanup corrections 留待 pre-1.0 release review。

Frozen patch SHA256：
`46b1ebad6e7a3de57aa19ef2f2451621c905c1b274c9f0945eca41e9f8ed41c7`。
Actual ios binary SHA256：
`b05e520c44307f686a9b0f3de2565a07627583f8056bd1da8f87c84f8e83c499`。
Actualcommit 等于 frozen tested patch；日志/元数据/audits 在 ignored
`target/validation/webinspector-server-setup-2026-10-05/`。RefsHEAD/status保持。

L01/class4/all31 active。其余 server startup/pinned signal/request/output/publication/
primary cleanup/revocation/join、其他 L01协议清理、L02–L04/Q04与第五至九类继续。
Cooperative timer 不证明 hard preemption/Apple ACK/rollback/retry/other-platform/
benchmark/RSS/decoderpeak。全部31任务/subtasks/history及118capability正文/maturity/
prior evidence保持，9完成/11未开始/11进行中或待条件不变，不缩减总体目标。

文档audit PASS：8documents/7Markdown/373locallinks与actualhelp118；all31
task/subtask/history prefix、118capability正文/maturity/prior evidence保持。
真实source/binary/16gates/full42886/current15 tests匹配，文档期间无product edit，
refsHEAD/status不变。实现和文档独立提交后继续 cleanup/server/其他工作，goal active。

### 大类 4：L01 direct CDP page cleanup（2026-10-05，整体进行中）

实现 `221a0c4fc6ed90e864694a3ec7d634808329bc52`，基于
`f10d0152204d4bb12b42ee38efb402410b48dc13`；明确6source/test files，213+/19-。
证据见 [CDP page cleanup](validation/cdp-page-cleanup-2026-10-05.md)。
Direct page loop 返回后，device guard/stream/page lock 持有到独立2s cleanup结束；
复用既有 session owner/control::finish，仅扩大 WebInspector 内的私有可见范围。
原 teardown 继承 active timeout 并吞掉结果；现在正常关闭可返回 cleanup failure，
已有主错误时保留两条cause与原primary分类/native来源。Error/timing migration
保留 pre-1.0 release review。仅 fully attached/usable 发既有 forwarding close，
invalid clients Drop，不猜恢复请求；local flush 不是 Apple ACK。

5 actual private page WebSocket/owned plist fixtures：正常关闭 native PermissionDenied、
JSON primary+native cleanup、真实默认2s Pending flush expiry、JSON+native Elapsed，
以及80ms flush 在50ms active idle policy之外完成。每项观察一次source call、既有
close frame、stream Drop/EOF且无额外请求。无product test route/public backend/forged
ConnectedDevice。原assertion functions、page/browser/setup active行为、shared cleanup/
controller logic不变；cfg(test) Source加 flush fault/counters，默认保持normal。
Core/public/bindings/access/dependencies不变；stored100ms/browser2s政策保持。

Before-product88897 terminal101/0PASS5FAIL；正常关闭错返Ok、双失败丢cleanup，
delayed flush未完成。修复后15775 terminal0/5PASS，无compile/test failure afterfix。
计时从close admission前开始，避免观察race。一次文档patch context mismatch未应用，
修正文档上下文后继续，未改product source。

Frozen pipeline81951 terminal0，全部17gates PASS：5/min5/setup15/page5/finite21/
shell31/CLI124/Core96+public5+integration1=102/coreonlycachemin17/minCLIalltargets/
originalstack5/strictCoreCLI/fmt/targetedtooling22/actualCLI95。第11节完整拆分PASS：
2441Rust/13Python，CLI836/Core1443/public52，0FAIL/2既有ignored。Workspacefmt/check/
strictClippy/MSRV1.80.1、66features/4groups、7+7doctests/3snippets、CABI、3.12wheel/
sdistbuild/install/import、tooling67/help118/raw38/host5/CLI95/backup3全部PASS。
22device NOT_RUN，Chromium/installed3.9/3.13未重跑。所有build终止后才改源码或启动
新验证；观察超时不等于退出。无cacheclean。

Frozen source patch SHA256：
`fcc622d1602b58f3fb7d5acfb4a9869a526c3366cf5857cb43a3c073b8780499`。
Actual ios binary SHA256：
`794a218696162073a0779d466d408d1fac534df1eb6431b6446a56f418862b03`。
Actual commit 精确匹配 frozen/tested patch；日志/元数据/audits 在 ignored
`target/validation/cdp-page-cleanup-2026-10-05/`，refsHEAD/status保持。

此路径只处理 returned bridge result；outer principal/server cancellation仍会Drop
handler，未证明取消后的异步cleanup或join。Constructor primary cleanup、active request/
output totals、HTTP publication、server signals、joined browser/stored owners继续。
其他 L01协议/清理、L02–L04/Q04与第五至九类仍保留。无remote ACK/rollback/retry/
other-platform/benchmark/RSS/decoderpeak/async Drop验收。All31任务/subtasks/history、
118capability正文/maturity/prior evidence保持；9完成/11未开始/11进行中或待条件
不变。实现和文档独立提交后继续其他cleanup/server工作，goal active。

文档audit PASS：8documents/7Markdown/379locallinks与actualhelp118；all31任务/
subtasks/history prefix及118capability正文/maturity/prior evidence保持。实际source/
binary/17gates/full81951匹配，docs期间无product edit，refsHEAD/status不变。

### 大类 4：L01 WebDriver runtime cleanup（2026-10-05，整体进行中）

实现 `8956866c4d095c5f3f0919a431b8b7e022f55e54`，基于
`37ab23affb041874b72cf5dc99cf9d41f662c989`；明确7source/test files，400+/83-。
证据见 [WebDriver cleanup](validation/webdriver-cleanup-2026-10-05.md)。Runtime.stop
借原AutomationSession/application identity，先context再forwarding socket，两步
共用stored100ms；原stop只关context，core10s依赖outer100ms截断。Shared Automation
cleanup主体原样抽为borrowed helper，不clone session/command IDs；finite/shell5s、
directpage2s保持。首次poll stop即disarm，成功/业务失败/caller cancel均不重试。
Native IO/Elapsed与双cleanup causes/原primary分类保留；invalid跳过socket恢复。
显式runtime Drop释放stream，不证明async Drop或Apple ACK。

8 actual constructor/privateSource/sharedAutomation plist fixtures覆盖正常close、
native PermissionDenied、context业务+socket IO、真实100ms pending flush expiry、
context80ms+flush30ms共用total、context未回导致invalid/norecovery、成功/业务失败
重复stop无额外wire，以及真实close frame后的caller cancel。IDs1/2/source1、
owned forwarding identity、stream Drop/peer EOF实际观察。Privatecfg helpers扩大
WebInspector内可见性，加default-off stopdelay/stall与commandID counters；原peer
逻辑/断言保持。仅一旧assertion迁移：成功WebDriver stop teardown0→1；其余所有
原assertions、active/setup/HTTP/CDP/controller/core/public/bindings/access/deps保持。

首次86097 terminal101，14 testvisibility compile errors，无tests运行；修正cfg可见性
后pre-product23646 terminal101/0PASS5FAIL。After-owner3719 terminal0/5PASS，
expanded12440 terminal0/8PASS；无post-product compile/test failure。一次JS audit
launcher typo在ignored script写入前修正，product source未改。

Frozen pipeline96261 terminal0，全部18gates PASS：8/min8/direct5/setup15/page5/
finite21/shell31/CLI132/Core96+public5+integration1=102/coreonlycachemin17/
minCLIalltargets/originalstack5/strictCoreCLI/fmt/targetedtooling22/actualCLI95。
第11节完整拆分PASS：2449Rust/13Python，CLI844/Core1443/public52，0FAIL/2既有ignored。
Workspacefmt/check/strictClippy/MSRV1.80.1、66features/4groups、7+7doctests/3snippets、
CABI、3.12wheel/sdistbuild/install/import、tooling67/help118/raw38/host5/CLI95/backup3
全部PASS。22device NOT_RUN，Chromium/installed3.9/3.13未重跑；所有build终止后才
改源码/启动新验证，观察超时不等于退出。无cacheclean，refsHEAD/status保持。

Frozen source patch SHA256：
`f821c0573294c3de72b9b4f545c567f63bd54ac7a62c2ced63c068115e398a57`。
Actual ios binary SHA256：
`10c40bafe12f2f1ec9eb11d80368710f22041a813d7a0f1f55458aea5efc6cae`。
Actualcommit等于frozen tested patch；证据在ignored
`target/validation/webdriver-cleanup-2026-10-05/`。

HTTP DELETE仍丢弃cleanup result，outer100ms可抢先截断helper；mutexwait admission/
remainder、active Arc races、registry revocation/join、constructor primarycleanup、
principal/servercancel、active request/output totals继续。DELETEgrammar/shape保持，
forwarding/timing correction保留pre-1.0 review/真机条件。其他 L01、L02–L04/Q04、
第五至九类/all31子任务保留；无ACK/rollback/retry/platform/benchmark/RSS/decoderpeak/
asyncDrop/join验收。118capability正文/maturity/prior evidence与9完成/11未开始/
11进行中或待条件不变；实现文档独立提交后继续owner工作，总体goal active。

文档audit PASS：8documents/7Markdown/385locallinks与actualhelp118，all31任务/
subtasks/history prefix、118capability正文/maturity/prior evidence保持；actualsource/
binary/18gates/full96261匹配，docs期间无product edit，refsHEAD/status不变。

### 大类 4：L01 direct CDP page lease ownership（2026-10-05，整体进行中）

实现 `5ecb565633179185c77ecce9a688eb148ad62235`，基于
`fded0b0e50b8a8d858babae3261ab04a3cfdb1b7`；明确3source/test files，399+/10-。
证据见 [CDP page lease](validation/cdp-page-lease-2026-10-05.md)。Direct page将
principal.revoked的一条pinned future放入setup/active owner；原outerPrincipal.run
丢ScopeDenied并直接Drop，绕过cleanup。Setup沿用checkedBudget.drive；active采用
biased outercontrol与每消息轮次yield，保留原innerunbiased流量选择。返回typed
OperationCancelled后等待既有独立2s cleanup，保留primary/native cleanup，不重复
poll已完成lease。Device/client/page lock持有到cleanup结束，invalid不猜恢复写。
撤销后停止业务处理，但连接可在cleanup中继续持有。现有safe diagnostic会记录
revocation，原outerwrapper静默；timing/diagnostic migration留待pre-1.0 review。

7新增tests：6actual cdp_router/admission/syntheticPolicy expiry/reload fixtures，
1private handler Cancelled/native IO cause observer。实际TargetCreated后观察
Runtime.enable frame证明已attach；close identity/source1/Drop/EOF/实际lock key
reacquire、80msflush独立于50ms active policy、观察2bytes之后partialcancel无恢复、
pending1sSource在返回前取消且无wire、真实2s Pendingcleanup不重poll signal均覆盖。
私有adapter核对授权syntheticDEVICE后仅委托ownedSource别名，不接真实设备/凭据。
Private收到字节counter/ownedstopfixture默认Pending；原五个lifecycle assertions、
traffic branch bodies、setupoperation/returnedcleanup/browser/otherowners/controller/
core/public/bindings/access/deps保持。Testlistener/peer join不是产品supervisor join。

Before-product24664 terminal101/1PASS3FAIL：partialcancelPASS，三个已attach撤销
均缺close frame。84618 terminal0/4PASS，expanded98632 terminal0/7PASS，无postfix
compile/test failure。一次sourcepatch context mismatch未应用；修正后继续，outer/
inner selection布局在首次post-product compile前定稿，原trafficchoice保持。

Frozen53535 terminal0，全20gates PASS：7/min7/WD8/direct5/setup15/page5/finite21/
shell31/CLI139/access29/Core96+public5+integration1=102/coreonlycachemin17/
minCLIalltargets/originalstack5/strictCoreCLI/fmt/targetedtools22/actualCLI95。
第11节完整拆分PASS：2456Rust/13Python，CLI851/Core1443/public52，0FAIL/2既有ignored。
Workspacefmt/check/strictClippy/MSRV1.80.1、66features/4groups、7+7doctests/3snippets、
CABI、3.12wheel/sdistbuild/install/import、tooling67/help118/raw38/host5/CLI95/backup3
全部PASS。22device NOT_RUN，Chromium/installed3.9/3.13未重跑；所有build终止后才
改源码/启动新验证，观察超时不等于退出。无cacheclean，refsHEAD/status保持。

Frozen source patch SHA256：
`d54f0345641659b337a1411bf35b0e9ad02afcee909d4201a7e76d8847bcb2ec`。
Actual ios binary SHA256：
`1ce215b1d7e9bb3c6e7e77a65fc925ffc56812527eb5fe09b96278b415543071`。
Actualcommit等于frozen tested patch；证据在ignored
`target/validation/cdp-page-lease-2026-10-05/`。

仅directpagelease owner。Genericcaller/serverDrop、constructorprimarycleanup、
active request/outputtotals、joinedbrowserchildren/storedsession owners继续。
其他L01、L02–L04/Q04与第五至九类/all31子任务保留；无Apple ACK/rollback/retry/
asyncDrop/join/platform/benchmark/RSS/decoderpeak验收。118capability正文/maturity/
prior evidence与9完成/11未开始/11进行中或待条件不变；实现文档独立提交后继续
browser/stored/server/其他owner工作，总体goal active。

文档audit PASS：8documents/7Markdown/391locallinks与actualhelp118，all31任务/
subtasks/history prefix、118capability正文/maturity/prior evidence保持；actualsource/
binary/20gates/full53535匹配，docs期间无product edit，refsHEAD/status不变。

### 大类 4：L01 CDP browser child outcomes / explicit joins（2026-10-05，整体进行中）

实现受测提交 `d8cf32eb4ad09217c3f9962df658eea03fed12e9`，parent
`79d008b165824b78bdb8d30dc51b5d3551daaf48`；详见
[browser task evidence](validation/cdp-browser-tasks-2026-10-05.md)。三个文件，
669增/76删，11个新测试；不完成L01、第四类或总体31任务。

| 范围 | 实际交付与验收 |
| --- | --- |
| Child stop/primary/cleanup | 一个 pinned local watch stop 在 active work 外层；每轮yield，内层traffic仍unbiased。usable empty idle恢复，invalidating timeout返回原错误。primary在独立2s cleanup前记录，cleanup在现有Closed event前记录；invalid client不猜恢复写入 |
| Explicit supervisor | detach/Closed/dead input明确stop和join，错误用现有safe diagnostics，保留成功detach回复与健康browser。正常/返回错误的browser退出，对全部剩余children共用一个2s总期限；先abort全部pending，再逐一join，某个失败不跳过其他join |
| Error ownership | 已观察primary在supervisor abort后保留；native/dual cleanup与实际发行Elapsed由既有wrapper持有，未知或未返回cleanup不伪造。unexpected native JoinError保留，正常结束缺cleanup outcome返回local Protocol |
| Original contracts | direct page、browser constructor setup body、discovery helpers、全部既有assertions保持；仅一个cfg Drop fixture新增required outcome字段。core/public/bindings/access/deps/其他owners/controller不变；无新selector、公开backend或credential/device操作 |
| Pending conditions | Browser outer Principal.run仍Drop整个parent，generic caller/server Drop不async join；被remove后正在cleanup的child须在raced active future之外保留所有权与join进度。constructor primary cleanup、active request/output totals、stored registry、其他L01、L02–L04/Q04及第五至九类继续 |

11个owned tests覆盖实际private constructor+plist stream：80ms flush大于50ms
active idle仍完成；真实default2s flush expiry；native IO；native nested JSON与
IO双错误；close frame证明primary先记录、50ms supervisor expiry后abort/join仍
保留JSON；两个pending children共用default2s；多轮empty idle恢复正确reply；
counter证明2bytes已消费后取消且不发close；实际64满queue与额外收到bytes时取消
并释放；原mpsc SendError；一个fixture内两个standalone panic/missing-outcome
actors分别与真实forwarding child共同stop，原JoinError/local Protocol保留且
另一个child仍close/join。standalone actors只验supervisor admission，不伪称设备。
实际app/page/sender、stream Drop/peer EOF、qualified lock reacquisition核对。
两children观察<3.5s区别serial4s，仅fixture判别，不是L04性能基线。现有terminal
try_send在满queue时仍可能丢Closed，join/release独立证明，不伪造terminal delivery。

历史negative25725 terminal101：0PASS/2FAIL，实际cleanup沿用50ms/80ms flush未
完成；corrected55684 terminal0：2PASS；expanded52734 terminal0：6PASS；
expanded15147 terminal0：11PASS。测试中的重复JoinHandle await在expanded编译
前修正，由supervisor独占join。之前观察超时未当退出；fmt与构建结束后才编辑。

Frozen pipeline52191 terminal0，全部21 gates PASS：11/min11、lease7/WD8/
direct5/setup15/page5/finite21/shell31、CLI150/access29/Core102（96+public5+
integration1）/core-min17、minimumCLIalltargets、原stack5、strictcoreCLIClippy、
fmt、.venv tooling22、binary与actualCLI95、第11节完整Rust/Python拆分。
完整命令 `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`，2467Rust/13Python PASS，
CLI862/Core1443/public52，0FAIL/2existing ignored。Workspacefmt/check/strict
Clippy、MSRV1.80.1、66features/4runtimegroups、7+7doctests/3downstreamsnippets、
CABI、Python3.12wheel/sdist build/install/import、tooling67/help118/raw38/host5/
CLI95/backup3均PASS。22device NOT_RUN，Chromium/installed3.9/3.13 NOT_RERUN；
PBKDF2与单独browsergate保留ignored，不升级maturity或假冒真机通过。

Actual frozen source patch SHA256
`bf4149bca9f6545e8b7021a9d1a004319a7f013939e7a4d61dc4d6ae0950d7f3`；
actual debug binary SHA256
`358870ad1afb7f3f2422ecca8a5c9fb08d7a49cedb2fecbfe622b1474dbceaeb`。
Commit与frozen source相等；durable logs/development原hash/audits位于ignored
`target/validation/cdp-browser-tasks-2026-10-05/`，refsHEAD/status不变。

所有31任务/subtasks/history保持，只L01overview与末尾记录追加；118capability
正文/maturity/prior evidence保持，只追加host entry。9完成/11未开始/11进行中
或待条件不变。无Apple ACK/rollback/retry/asyncDrop/browser lease join/platform/
benchmark/RSS/decoder peak认证。实现后独立文档提交，再继续browser租约撤销
期间retiring owner/join、stored/server与其他owner，L01/class4/总体goal active。
无subagent/push/publication/history rewrite/reference edits/device/Apple操作。

### 大类 4：L01 CDP browser lease / retiring owners（2026-10-05，整体进行中）

实现受测提交 `d73514f4b83c956fe860f1051d27f0d912203a5d`，parent
`36d5a21f1d7e43926994bb7893bae8ad8f7e398a`；详见
[browser lease evidence](validation/cdp-browser-lease-2026-10-05.md)。4files，
873增/70删，12new tests（child新增3、browser新增9）；不完成L01/第四类/总体。

| 范围 | 实际交付与验收 |
| --- | --- |
| Parent lease | 一个pinned principal.revoked贯穿initial discovery与active work；biased outer control/yield每轮保留inner unbiased traffic。原outerPrincipal.run Drop改为typed Cancelled返回并继续独立cleanup，既有safe diagnostic开始记录browser撤销 |
| Retiring ownership | 每个被remove的child在cleanup await前进入raced active future之外的retiring owner。借用stop被取消仍保留原deadline、completed native join与已发行wait error；重启不重新poll已完成JoinHandle，所有outcome在全部join后无await地一起移交 |
| Common cleanup | parent退出将retiring与remaining children合入一个独立2s group；此前已发行错误保留。先abort全部pending，再逐一join，任何失败不能跳过其他child；parent Cancelled保持primary，原child/waiter/native cleanup保持owned |
| Prior contracts | direct page、actual browser child constructor/active/cleanup、discovery helpers、全部既有assertions与提取的release assertions保留；parent traffic只有retained borrowed cleanup修改。core/public/bindings/access/deps/other owners/shared controller不变；private测试wrapper/import加cfg(test)，无新wire/backend |
| Conditions | generic caller/server Drop仍仅abort，没有async join；constructor primary cleanup、active request/output totals、stored registry、terminal event retention、其他L01/L02–L04/Q04及第五至九类全部继续。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN |

9browserfixtures中8个经actual cdp_router/admission与synthetic policy的browser
WebSocket：reload/expiry附着后matching close；80ms flush在reload后完成；
counter证明2bytes partial已消费后不发recovery；pending flush使用真实default2s；
detach actual close frame且flush未完成时reload，retiring owner仍使flush完成；
initial Source4s delay取消前Return/wire未发生、1call/0receivedbytes/DropEOF；
child Source4s delay取消未发布owner、2calls/0receivedbytes/qualified page lock
释放。QueueSource核对authorized syntheticDEVICE再调用既有owned source alias；
实际autoAttach+Runtime.enable forwarding证明attached。另1private result observer
只验actualhandler/ownedstop保留typedCancelled与原PermissionDenied IO，不宣称
新的admission/revocation。Listenerfixture的join不认证productserver supervision。
Expiry若contact额外pending periodic Source则核其Drop/EOF，未contact不冒充释放。

新增3childfixtures：部分completed join后drop借用cleanup并继续不重复poll；
JSON/IO双错误先完成时继续等待另一个pending child，错误保留；真实50ms Elapsed
发行后abort/join期间取消，reparent仍保留原Elapsed并收尾。旧11childfixtures
继续PASS（含shared2s、native primary afterabort、panic/missing outcome/full queue）。
现有terminal try_send仍可能丢满queue的Closed，不伪造event delivery或AppleACK。
Cooperative timer不preempt同步work；generic Drop/server/性能与decodepeak仍待。

61458 terminal0：child14PASS。34143首次compile terminal101：4fixture E0283/
E0608、未运行测试；修正68558 terminal101：1PASS/5FAIL，partialcancel通过，
4close缺失/1detach flush未完成。Parent改后90464 terminal0：20PASS；expanded
11501 terminal0：23PASS。尝试helper visibility的patch因既有helper已可见未应用，
并未编辑原helper。首次frozen10010 terminal101：前15 gates PASS，strictClippy
因test-only constant import未cfg FAIL，full尚未启动。确认exit且无live build后
仅加cfg(test)，与archive first source比较唯一改动即该import guard。

Corrected frozen pipeline26582 terminal0/all21 gates PASS：23/min23、pagelease7/
WD8/direct5/setup15/page5/finite21/shell31、CLI162/access29/Core102（96+public5+
integration1）/core-min17、minimumCLIalltargets、原stack5、strictcoreCLIClippy、
fmt、.venvtools22、binary/actualCLI95与第11节full Rust/Python拆分。完整命令
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`。
2479Rust/13Python PASS，CLI874/Core1443/public52，0FAIL/2existing ignored。
Workspacefmt/check/strictClippy、MSRV1.80.1、66features/4runtimegroups、7+7docs/
3downstreamsnippets、CABI、Python3.12wheel/sdist build/install/import、tooling67/
help118/raw38/host5/CLI95/backup3均PASS。Build退出之前无source编辑或重复validator；
观察超时未当退出，旧failed attempt/development logs原hash保留。

Actual frozen source patch SHA256
`9dfc360a55fc161b621a6352fa61d0d5c96cc69692052f175d083e1085ef517f`；
actual debug ios binary SHA256
`64c6adcef589a9a6c20cd71934a08de79193aed63ea31a9545b3a46b00bddaf7`。
Commit=frozen source；durable evidence/audits/failed attempt位于ignored
`target/validation/cdp-browser-lease-2026-10-05/`，referenceHEAD/status不变。

All31 tasks/subtasks/history精确保留，只L01overview/末尾记录与单项host evidence
追加；118capability正文/maturity/prior evidence、9完成/11未开始/11进行中或待
条件保持。Timing/diagnostic correction保留pre-1.0 release review与device条件。
无Apple ACK/rollback/retry/asyncDrop/server join/platform/benchmark/RSS/decoder
peak认证。实现后独立文档提交，继续WebDriver DELETE清理错误、stored registry
任务join/server与其他owners，再其他L01/L02–L04/Q04及第五至九类，goal active。
无subagent/push/publication/history rewrite/reference edits/device/Apple操作。

### 大类 4：L01 WebDriver DELETE total / cleanup errors（2026-10-05）

受测实现：`731bdb0ac96d30013a17c083fd7c9f19c8a85a81`，基于
`e188f1f8bbf06b7a4c266ef05c75c580258c76f9`。Linux x86_64，Rust1.97.1/
MSRV1.80.1，Python3.12.13。[专题证据](validation/webdriver-delete-2026-10-05.md)。
只交付 DELETE handler 主机范围；L01/第四类/全部31项继续 active。

DELETE handler entry 创建一个 checked100ms Budget，map removal、runtime mutex
等待、原 Automation context close 和 forwarding close 共用同一绝对 deadline。
先移除地址能力再协议I/O；原session/command IDs保持，cleanup开始disarm一次，
invalid client跳过。Result helper不以外层竞争timer覆盖已知business主错误；
shared cleanup body只增加private visibility。原runtime.stop、unit revocation
callback、constructor/active methods与八个既有cleanup assertions不变，fixture
只增加cfg visibility。Core/public/access/registry/dependencies/其他owners不变。

此前 outer timeout 和 unit callback 隐藏失败，DELETE仍返回200/null。现在经既有
AppError500/safe unknown-error/schema1 machine headers报告原PermissionDenied、
Elapsed/Timeout或business Protocol主错误，双causes保留到HTTP projection。
成功/重复DELETE仍200/null无错误headers。期限不包含先行HTTP授权middleware或
response delivery；failure200→500与timing修正保留pre-1.0 release review/真机条件。

九个新测试：六个actual protected POST/DELETE覆盖normal/repeat、native flushIO、
real default100ms expiry、business primary+expiry、mutex占满预算零cleanup请求、
观察removal后85ms mutex等待不足完成30ms forwarding flush；三个private actual
Result observers保留business+nativeIO、business+Elapsed与mutex Elapsed，不作
HTTP/admission声明。源调用1、原create/close IDs1/2、close identity、Drop/peer/
listener join被观察；fixture Arc显式释放不代表取消所有in-flight requests，
fixture joins不代表product registry/server监督，close/flush不代表Apple ACK。

首编译12436 terminal101：四个Response.json fixture E0599，无tests；依现有
bytes+serde_json修正，不改reqwest feature。修正后before-product log 1PASS/5FAIL，
五项均200而非500；该次numeric handle/exit在compaction后缺失，如实未填精确
exit code。确认最终FAIL log与无live builds后才编辑源码。45035 terminal0：6PASS；
40794 terminal0：9PASS。Frozen pipeline91290 terminal0，全部22gates PASS。

9/min9、browser23/pagelease7/WDcleanup8/direct5/setup15/page5/finite21/shell31、
CLI171/access29/Core102/core-min17、minimumCLIalltargets/原stack5/strict Clippy/
fmt、.venv tooling22/binary/actualCLI95及§11 full全部PASS。完整入口按Rust/Python
拆分：2488 Rust/13 Python，CLI883/Core1443/public52，0FAIL/2既有ignored。
Workspace fmt/check/strict Clippy/MSRV、66features/4groups、7+7docs/3snippets、
CABI、Python3.12wheel/sdist build/install/import、tooling67/help118/raw38/host5/
CLI95/backup3 PASS；22device NOT_RUN，Chromium/installed3.9/3.13未重跑。

Source patch SHA256：
`c0d345de35ad32ba9d86bb8f62504f4f76e50fbaf2c12143b72da480a61cd54a`。
Actual ios binary SHA256：
`b0d4a5a156df6c9751d3ad3c8db4a06a97992ef9b4a95d3c6a7627423f89c3fb`。
Commit=frozen source；durable history/logs/audits在ignored
`target/validation/webdriver-delete-2026-10-05/`，referenceHEAD/status不变。

全部31项/子任务/history、118capability正文/maturity/prior证据与9完成/11未开始/
11进行中或待条件保持。Stored registry join/revocation callback/active Arcs、
generic caller/server Drop、constructor primary cleanup、active request/output
totals、terminal retention与其他L01/L02–L04/Q04/第五至九类继续。无Apple ACK/
rollback/retry/asyncDrop/server join/platform/benchmark/RSS/decoder peak认证。
代码后独立文档收尾提交，再继续任务所有权等可执行范围；goal active，
无subagent/push/publication/history rewrite/reference edits/device/Apple操作。

### 大类 4：L01 stored session task owners / explicit joins（2026-10-05）

受测实现：`7fb827b1ee4ff17e29597123b23672184a20a0a5`，基于
`a0bf070aa52d14c57c1ee1981c0b5acdf2c6ff06`。Linux x86_64，Rust1.97.1/
MSRV1.80.1，Python3.12.13。[专题证据](validation/session-store-2026-10-05.md)。
只交付stored watcher所有权/join主机范围，L01/第四类/全部31项继续active。

Store持有active entries和retiring ArcTaskOwners。显式remove/replacement先把
owner保留并发出abort，再等待join mutex；取消该等待不丢owner/abort请求，
borrowed join记录Normal/RequestedAbort/Unexpected和原nativeJoinError，完成
handle不再poll，原panic/externalcancel分类Unknown/Cancelled，显式abort的
cancelled join为预期结果。转移native error后无额外await，registry经既有safe
tracing报告，不增加HTTP fields/raw panic。DELETE既有100ms预算包含map join。

Revoked actor核对original weak resource identity再移除条目，把自身handle交
Store retirement，释放strong store/global lock后运行既有100ms unit cleanup
callback；不selfabort/selfjoin，旧actor不能移除replacement资源。Registry
insert/get/principal/remove只reap finished actors，try_lock不等active join，
protocol cleanup/task join都在全局表锁外。无新actor/background reaper/source
backend/protocol operation。Idle completed handles仍等下一次registryAPI/Drop，
generic Drop仍fallbackabort而非asyncjoin。Timing迁移保留release review/真机条件。

四个文件687+/22-，13new tests：七个ownedTokio actors验证normal/repeat/abort/
原panic/externalcancel/cancelresume/nonblockingreap/Drop；六个protected actual
loopback HTTP registry inserts与private Sessions操作使用generic owned host
Resource，验证remove先join、joinmutex占用时取消保留+另一真实HTTP入表、reload
80ms正常退出不selfabort、actualdefault100ms pending callback后normaljoin、
sameIDreplacement保留新generation、Drop前nativeAbortHandle证明actor仍pending/
资源未释放且weakstore随后失效。它们不是Automation wire/device fixtures，
fixturelistenerjoin不代表productserverjoin。原两个securityscope/Drop assertions
及middleware逐字保留，其他owners/core/public/access/dependencies不变。

首次99572 terminal0：13PASS；之后补native分类/Drop前actualpending proof，
冻结gates验证latestsource。首frozen63290 terminal101：18preceding gatesPASS，
strictClippy manual_inspectFAIL，fullNOT_STARTED。确认真实exit/no live builds后
只改remove.map→inspect并去冗余entryreturn；archived四文件bytes/SHA和唯一修正
已审查。Corrected82353 terminal0/all24gates PASS，源码冻结至权威退出。

13/min13、prioractualDELETE9/oldscope2/browser23/pagelease7/WDcleanup8/direct5/
setup15/page5/finite21/shell31、CLI184/access29/Core102/core-min17、minimumCLI
alltargets/原stack5/strictClippy/fmt、.venv tooling22/binary/actualCLI95/§11full
全部PASS。完整Rust/Python拆分2501Rust/13Python，CLI896/Core1443/public52，
0FAIL/2既有ignored；workspacefmt/check/strict/MSRV、66features/4groups、7+7docs/
3snippets、CABI、Python3.12wheel/sdistbuild/install/import、tooling67/help118/
raw38/host5/CLI95/backup3 PASS。22device NOT_RUN，Chromium/installed3.9/3.13未重跑。

Source patch SHA256：
`05695680f3810a98b850c45eba0984130e406afdf334c9525e6e68d5e9212a34`。
Actual ios binary SHA256：
`f22253382855a70083544b0e29d4ef00d4787679fcaa5b346767f939620274a7`。
Commit=frozen source；durable firstrun/correctedgates/archivedfailure/4sourcebytes/
nativehashes/audits在ignored `target/validation/session-store-2026-10-05/`，
referenceHEAD/status不变。

原unit callback/outer100ms尚需native protocol cleanup主错误保留，active request
Arcs、idle retired handles无API回收、caller/store/serverDrop、constructorprimary
cleanup/active totals/terminal retention/resourcecaps仍保留；其他L01/L02–L04/Q04/
第五至九类继续。全部31项/子任务/history、118capability正文/maturity/prior证据与
9完成/11未开始/11进行中或待条件保持。无Apple ACK/rollback/retry/asyncDrop/server
join/platform/benchmark/RSS/decoder peak认证；本地代码后独立文档收尾提交，
总体goal active，继续余下owners与manager等可执行范围。
无subagent/push/publication/history rewrite/reference edits/device/Apple操作。

### 大类 4：L01 WebDriver typed revocation cleanup results（2026-10-05）

受测实现：`bf2da2dcef7106a16b19f0bd355e80b0e998b642`，基于
`f306e1228edfde78a0c9180f4be10aa580781101`。Linux x86_64，Rust1.97.1/
MSRV1.80.1，Python3.12.13。[专题证据](validation/webdriver-revocation-2026-10-05.md)。
L01/第四类/全部31项继续active；本次只交付typed callback/returned-result范围。

实际WebDriver registration直接传existing stop_selenium_runtime_until给
insert_with_budget。Actor在条目retirement后创建一个checked100ms Budget，
runtime mutex/original Automation context/forwarding cleanup共享绝对deadline，
无外层竞争timer。原业务Protocol主错误不会在pending forwarding flush expiry
时丢失，原PermissionDenied/Elapsed及双causes保留经NativeJoinHandle<Result<()>>
和TaskOwner.join/reap转移，无后续await。Native Normal exit可携带cleanup Err，
不代表成功；unexpected nativejoin flags/classification保持，issued abort cancel
中性。既有safe error_message tracing label扩为automation session closed with
error，不增加HTTP/schema/route/status/success/API字段。Timing/diagnostic修正
保留pre-1.0 release review/真机条件。期限不包含先行map wait/server shutdown。

六文件380+/20-、八新测试。去旧unit log adapter；legacyunitinsert仅cfgtest
Budget.drive适配原hostfixtures，runtime.stop/durationcleanup wrapper只加cfgtest
而函数体不变。原DELETE/stop_until/typedmutexhelper/sharedprotocol cleanup、
constructor/active methods、retirement/admission/全部assertions保留；原Tokio
helper只补Ok以匹配nativeResult。Core/public/access/deps/其他owners不变，无新
task/source/backend/protocol operation。已知主错误在callback尚未返回时若store
abort仍待处理，不能宣称覆盖generic asyncDrop/全部request Arcs。

八fixture均actualprotectedproduct selenium_router POST+ownedAutomationpeer，
七个actualHTTP/auth/reload和一个actualGrantExpiry3s；private observe消耗实际
actor nativeResult，非替代cleanup/生产日志capture/serverjoin。覆盖normal/expiry
original context+forwarding close、nativeflushIO、realdefault100ms Elapsed、
businessprimary+nativeIO/business+Elapsed、mutex占满预算zero cleanup/armedflag
未disarm、stalledcontext invalidclient跳过forwarding recovery。原IDs1/2、matching
close counts、calls1/Drop/peer trace/fixturelistenerjoin与safeformatter无synthetic
DEVICE/TOKEN/session泄漏被观察；fixtureArc显式释放不代表取消所有inflight requests，
close/flush不是Apple ACK/设备证据。

首82798 terminal0：8PASS；首frozen94556 terminal101：19precedinggatesPASS，
strictdeadcodeFAIL仅剩cfgtest使用的private durationwrapper，fullNOT_STARTED。
确认权威exit/no live builds后唯一修正为wrapper加cfgtest/body不变，archived5
source bytes与sharedowner原baseline及精确修正已审查。Corrected98957 terminal0：
all25gates PASS，源码冻结至真实进程退出，观察超时不算退出。

8/min8、store13/DELETE9/oldscope2/browser23/pagelease7/WDcleanup8/direct5/setup15/
page5/finite21/shell31、CLI192/access29/Core102/core-min17、minimumCLIalltargets/
原stack5/strictClippy/fmt、.venv tooling22/binary/actualCLI95/§11full全部PASS。
完整Rust/Python拆分2509Rust/13Python，CLI904/Core1443/public52，0FAIL/2既有ignored。
Workspacefmt/check/strict/MSRV、66features/4groups、7+7docs/3snippets、CABI、
Python3.12wheel/sdistbuild/install/import、tooling67/help118/raw38/host5/CLI95/
backup3 PASS；22device NOT_RUN，Chromium/installed3.9/3.13未重跑。

Source patch SHA256：
`8466c0b69a65828974ee3b14a72c7ce513a410a6b2b95a6aee27a609d1d43592`。
Actual ios binary SHA256：
`9a8b97ff270f422e9585129b4b6432812be812f0db976282d0ac4bcfdcc6e513`。
Commit=frozen source；durablefirst/correctedgates/failedattempt/sourcebytes/native
hashes/audits在ignored `target/validation/webdriver-revocation-2026-10-05/`，
referenceHEAD/status不变。

先行map wait/knownprimary beforecallbackReturnwhenStoreabort、activeArcs/idle
completedhandles/caller-serverDrop/constructorprimary/active totals/terminal-caps
与其他L01/L02–L04/Q04/第五至九类继续。全部31项/子任务/history、118capability
正文/maturity/prior证据及9完成/11未开始/11进行中或待条件保持，无Apple ACK/
rollback/retry/asyncDrop/serverjoin/platform/benchmark/RSS/decoderpeak认证。
代码后独立文档收尾提交，持续推进其余L01和manager隔离等独立范围，总体goal active。
无subagent/push/publication/history rewrite/reference edits/device/Apple操作。

### 大类 4：L02 CLI manager device isolation（2026-10-06）

受测实现 `e0f52671396e6408fc38921f78f73a256a45ce65`，父提交
`4afde1f5288e2ebec22577f0c07f62d9a6edfbe2`。源码2026-10-05冻结、启动验证，
2026-10-06恢复持久化证据并审查提交。[主机证据](validation/tunnel-isolation-2026-10-06.md)
及 [manager 使用合同](tunnel.md) 记录实际边界。

CLI TunnelAgentState移除覆盖网络的全局生命周期锁。本地list/get/delete只用短时
table锁；每设备serial gate、最多4个共享connect permits。停止在publication锁内
撤销原有active/queued generations，旧结果不能在stop之后发布；现有200/404
语义保留，absent404也可取消pending start，其502保留原生OperationCancelled。
后续explicit start允许，后续scan可重新创建；不是永久设备排除。manual start
在取得ticket后共用15s queue/permit/native connect/existing-trust discovery/fallback/
publication total；此前queue无界、不同connect各15s。实际Elapsed、unmatched
fallback原IO保留；期限/类别修正待pre-1.0 review。

私有NativeSource调用相同4个Core connect/options与既有discovery；SDK/public/
deps/协议不改，cfg fake owned device只用于真实CLI manager/两项protected HTTP。
native devices private Box是实际额外allocation，不宣称性能优化。扫描单独串行，
共享同设备gate/permit，先capture全部候选ticket；stop during discovery拒绝旧snapshot，
旧list failure只清除原publication identity，保留等待期间新建记录。30/60/120/240/
300s backoff保留。扫描候选建连仍sequential；in-flight detach/replug identity、
scan total、native worker joins/资源与真机继续。

四源码文件1133+/303-、15新测试；原tunnel assertions/HTTP/auth/federation/bridge/
record语法与core/public/access/deps BYTE保留。first7782 terminal101，private
MuxDevice两visibility+两inference错误、无测试；仅改私有source UDID列表而不扩SDK。
21164 terminal0十三PASS，再加入native downcast/callerDrop及加强旧/新publication断言。

父handle9635恢复时MISSING，无livebuild；父tool exit UNKNOWN，不能虚构terminal0。
持久化14个subprocess均记录exit0，full最终PASS，冻结assert后已写binary hash；
恢复source/result audit terminal0。实际主线 **2524Rust/13Python**、CLI919/Core1443/
public52、0FAIL/2既有ignored。所有15/min15/tunnel56+1ignored/WebInspector192/
access29/Core102/minCLIalltargets/stack5/strict/fmt/tools22/binaryCLI95/full PASS。
66features/4runtimegroups/MSRV1.80.1/7+7docs/3snippets/CABI/3.12wheel-sdist/
tooling67/help118/raw38/host5/CLI95/backup3 PASS，**22device NOT_RUN**；
Chromium/installed3.9/3.13 NOT_RERUN，fakeDeviceDrop不替代native async joins/真机/
吞吐/RSS/socket/decoder峰值验收。Ignored evidence path:
`target/validation/tunnel-isolation-2026-10-05/`，reference HEAD/status不变。

source patch SHA256 `97fda9f0acc5ca57d037a3362227e141d42792ed0d5ccec17da595c37d51bc99`；
actual debug ios SHA256 `871daaa54afaf6275a14bf490d1cef7d58783bf63db0e653317496645e444917`；
commit=frozen。L02由待开始改进行中，真实总览9完成/10待开始/12进行中或待验收条件。
全部31项/全部子任务/历史、118capability正文/maturity保留，仅增host evidence。

L02所需Coreconnect/RSD/H2与S01主机接口已就绪，L01剩余owners不因此完成。
接着bounded parallel auto scans/in-flight identity/scan totals/resource，再L03–L04/Q04
及其他L01和第五至九类。L01/L02/第四类/总体目标active；无subagent/push/
publication/historyrewrite/referenceedit/设备或Apple凭据操作。

### 大类 4：L02 bounded parallel automatic scans（2026-10-06）

受测实现 `66bf7311020763b8a29b175a117cdf1b70d4e22f`，父
`b96ede822e52611d2be97be624baacb2317af608`。三源码319+/17-、八新测试；
[并发扫描证据](validation/tunnel-parallel-2026-10-06.md) 和 [manager 合同](tunnel.md)
记录当前行为。此前 e0f5267 profile 的 sequential 条件由本项实现解除，历史保留。

实际CLI扫描把usbmux与existing-trust mobdev2候选合并去重，保留firstUSB/
firstpairedhost的有效优先顺序。父scanner future用buffer_unordered直接拥有最多4个
建连future，不创建detached连接task；共用既有4permit/perdevice serial/watch/15s
candidate/error/backoff。A停滞时USB或mobdev2 B可在假设备预算1s内发布；
stopA不取消B。16候选只有4active/peak，其余12未接入；parent Drop全部释放
source futures/permits。Fake device/future Drop不是Core worker join/Apple ACK/RSS。

既有paired discovery改为connect attempts之前，以解除USB batch阻塞mobdev2；
其等待可延迟USB启动，discovery/wholeScan total仍待，不声明真实吞吐改善。
paired discovery原错误保留至USB候选处理后返回，成功USB记录仍发布、ready不新设；
generic parent Drop期间已知错误更强保留属于剩余owner边界。NativeSource/Lifecycle/
refresh_connect及manual/table/HTTP/auth/federation/bridge/record/core/public/access/deps
BYTE不改。旧queuedB fixture从1stalledA调整为4stalledA/C/D/E并放行4permit；
原断言全部保留，其他旧test bodies不改；private fake增paired_failure。

53715 terminal101 **0PASS/5FAIL**，25737 terminal0 **20PASS**；新增cross-family
control88793 terminal101 **0PASS/1FAIL**，原mobdev2仍被USB batch阻塞；
合并窗口/补充error-route回归99805 terminal0 **23PASS**。日志和原process statuses/
hash保留，历史source preimages未存档，不虚构历史源码hash。冻结 **87432 terminal0**，
all14 gates **PASS**；audit terminal0，实际 **2532Rust/13Python**、CLI927/Core1443/
public52、0FAIL/2oldignored。23/min23/tunnel64+1ignored/WebInspector192/access29/
Core102/minCLIalltargets/stack5/strict/fmt/tools22/buildCLI95/full PASS。66features/
4runtimegroups/MSRV1.80.1/7+7docs/3snippets/CABI/3.12wheel-sdist/tooling67/help118/
raw38/host5/CLI95/backup3 PASS；**22device NOT_RUN**，Chromium/installed3.9/3.13
NOT_RERUN。观察超时不算退出；源码直到原进程authoritative completion保持冻结。

source patch SHA256 `1cb613e6083c8c68f134628c3fd9cbc11194381732cfa7c0d108fdf26a95da39`；
actual debug ios SHA256 `188538ef87689ab47b29392c56f757e5905890c25de576ed4a0f28e58d40590c`；
commit=frozen，ignored evidence `target/validation/tunnel-parallel-2026-10-06/`，
reference HEAD/status不变。调度/启动时机变化须pre-1.0 review。

L02仍进行中，9complete/10notstarted/12inprogress-orconditions/all31主项/子任务/
历史、118capability正文/maturity保留。接着in-flight detach/replug身份/旧attached
snapshot、discovery/wholeScan budget、metadata/resource/nativeworker/serverjoins，
以及其他L01/L03–L04/Q04和第五至九类。有限4permit不限制所有Core tasks/allocator；
无平台/真机/ACK/nativejoin/benchmark/RSS/decoder peak验收，无subagent/push/
publication/historyrewrite/referenceedit/设备或Apple服务操作；第四类/总体goal active。

### 大类 4：L02 observed attachment identity（2026-10-06）

受测实现 `1eb45c25145f5306a5831df6a0c29d494a94ca96`，父
`dac32cb6111205392051465027572e0eb4519feb`。四源码254+/18-、七新测试；
[身份快照证据](validation/tunnel-identity-2026-10-06.md) 和 [manager 合同](tunnel.md)
记录当前范围。Source保留现有MuxClient list已返回的UDID/deviceID/USB标志；
不扩SDK、不加protocol frame/daemon query。4Nativeconnect/options/paired discovery/
device adapter不改。实际CLI按observed(UDI,ID)清除原publication旧usbmux记录，
同UDI还在但ID变了也退役；其他仍attached的transportID和并发newpublication保留。
ManagedTunnel记录route family，代替ID0 sentinel；synthetic0不是wire validity证据。

candidate选择保持Core firstUSB/lastnonUSB及firstUDI position；期望ID与完成device
ID不符则drop结果、localOperationCancelled、无failure backoff，下次新快照可立即接入。
普通原IO/退避不改。serial/watch/permits/15s/window/HTTP/auth/record/core/public/access/
deps保持；原23assertionfunctions BYTE不改。manual expectedID仍None，未加postconnect
query/listener；matchingoldID未观察拔插/IDreuse/daemonrestart/快照后变化仍待，不能
根据ID equality认证physical freshness/Apple ACK/native worker join/RSS。退役/取消
语义变化待pre-1.0 review；继续更强发布新鲜度和完整in-flight身份，不把本profile当L02完成。

control95029 terminal101 **4PASS/2FAIL**（replug保留旧ID、stale candidate接纳新代），
临时metadata未使用warning不算严格验收；95472 terminal101 E0282 index inference/
无测试。原进程退出后唯一HashMap<String,usize>annotation修正，38985 terminal0
**29PASS**，之后增第7个synthetic0 route-family fixture再冻结。冻结
**26331 terminal0/all14 gates PASS**，audit terminal0，实际 **2539Rust/13Python**，
CLI934/Core1443/public52、0FAIL/2既有ignored；30/min30/tunnel71+1ignored/
WebInspector192/access29/Core102/minCLIalltargets/stack5/strict/fmt/tooling22/buildCLI95/
full PASS。66features/4groups/MSRV1.80.1/7+7docs/3snippets/CABI/3.12wheel-sdist/
tooling67/help118/raw38/host5/CLI95/backup3 PASS；**22device NOT_RUN**，
Chromium/installed3.9/3.13 NOT_RERUN。历史source preimages未存档，不编造历史hash；
观察超时不当退出，source直到authoritativecompletion冻结。

patch SHA256 `8a1a082a7fc834f0e63c469af60d4d5089aacef826944205376c1e2da45cc467`；
actual debug ios SHA256 `ad99790c2598d507bc3457c43c75f14415b770468d946ad3280230c8c21c393a`；
commit=frozen，ignored evidence `target/validation/tunnel-identity-2026-10-06/`，
reference HEAD/status不变。actualCLIstate ownedFakeMetadata independently controls
snapshot/completedIDs；不替代Corefake lock microtests，不代表真机/worker/socket/RSS。

L02/第四类/goal active，9complete/10notstarted/12inprogressconditions/all31主项/子项/
历史/118capability正文/maturity保留。接着publication freshness/manualinflight/
discovery-wholeScan totals/metadata/resource/nativeworker-serverjoins，再其他L01/
L03–L04/Q04及第五至九类。无subagent/push/publication/historyrewrite/referenceedit/
设备或Apple服务操作。

### 大类 4：L02 attachment query before publication（2026-10-06）

受测实现 `86fecfe3331e9efa74dc76fbc48b47ad5c3df8f1`，父
`12e6b17757a99f66ff506835712560a2c88ffbe4`。三源码198+/3-、六新测试；
[发布前查询证据](validation/tunnel-publication-2026-10-06.md) 与 [manager 合同](tunnel.md)
记录范围。生产只增加17lineblock：initial expectedID/recordvalidation后、
publication table lock之前，对Usbmux/directUSB结果再调用existingSource.usb_devices。
当前(UDI,ID)在则发布，absent释放owneddevice/localcancel；Connect/ListErr保留原
Anyhow/IO与safe static context。结果对象在query期间仍owned，分享原15s/serial/
permit/watch，无globalnetworklock；Wifi/mobdev2跳过，initial mismatch不发额外query。

实际增加ONEdaemon connection/list roundtrip，manual同样参加，可增加启动耗时，
待pre-1.0 review/platform/device evidence。reply snapshot之后变化/IDreuse/daemonrestart/
nativealivefalse/eventlistener/worker-serverjoins仍未解决，不能claim fullphysicalfreshness/
ACK/资源/RSS/benchmark。ordinaryIO/backoff保留；absent local cancellation不加新退避。
旧failedattempt UID-only仍可把71失败退避带到fresh72，下一项须继续，不宣称L02完成。

privateFake成功legacyUSB结果注册进default.USBlist（explicitdetails仍独立控制），
usb_calls计数、error在gate之前捕获为per-request outcome。唯一旧failed_scan
fixture改setup顺序：oldA先成功，再设旧failure/gate，poll旧query后设新query健康/
无gate再连B；保留全部旧assertions及其他30旧bodies；不加production bypass。
nativeSource/lifecycle/Core/public/access/deps/原production所有body除17block BYTE保持。

control26945 terminal101 **0PASS/6FAIL**，post14072 terminal0 **36PASS**；
冻结 **20077 terminal0/all14 gates PASS**、source/result audit terminal0。实际
**2545Rust/13Python**、CLI940/Core1443/public52、0FAIL/2既有ignored；
36/min36/tunnel77+1ignored/WebInspector192/access29/Core102/minCLIalltargets/stack5/
strict/fmt/tooling22/buildCLI95/full PASS。66features/4groups/MSRV1.80.1/7+7docs/
3snippets/CABI/3.12wheel-sdist/tooling67/help118/raw38/host5/CLI95/backup3 PASS；
**22device NOT_RUN**，Chromium/installed3.9/3.13 NOT_RERUN。观察timeout不算退出，
source直到authoritative completion冻结；真实coreworker/device并未由fakeDrop认证。

patch SHA256 `ed585c77da2af2dbcd7dcd9e6cef4d0370eabfcb0c65552e4d46bfa6f83432fc`；
actual debug ios SHA256 `0bf5bce817434a2c0c9af4679f2e68f246166d1b05636c07d80fa33c0176eaee`；
commit=frozen，ignored evidence `target/validation/tunnel-publication-2026-10-06/`，
reference HEAD/status不变。ActualCLIstate independently controlledsource/nativeID/query/
ownedresult/IO/Elapsed/stop/anothertransport，未用SDKfakeconstructor或新的protocol keys。

L02/第四类/goal active，9complete/10notstarted/12inprogressconditions/all31/全部子项/
历史/118capability正文maturity保留。接着generation/route-specific failurebackoff、
afterquery变化/未知manualepoch/reuse/restart/nativeLive/metadata/resource/wholeScan，
再其他L01/L03–L04/Q04和第五至九类。无subagent/push/publication/historyrewrite/
referenceedit/设备或Apple服务操作。

### 大类 4：L02 failure backoff by known connection identity（2026-10-06）

受测实现 `d0b04bed79ecec5c3f724ff003122f66b1159216`，父
`cef374bdbc583004e53b0a40193ca69886ea52eb`。三源码203+/4-、五新测试；
[连接身份退避证据](validation/tunnel-backoff-2026-10-06.md) 与 [manager 合同](tunnel.md)
记录行为。此前UID-only backoff会把71/旧pairedhost失败延迟带给fresh72/knownnewhost；
现在privateFailedAttempt保存Route+expectedNativeID，仅同identity才skip或增加count。
knownID/host/route变化可立即尝试，新identity失败从count1开始，成功照旧remove记录，
sameidentity原30/60/120/240/300s/time函数保持。privateidentity Debug只出staticlabel，
不泄露retainedhost。无需新pairing/actor/wire/API/deps，不改native connection/trust，
不重试安装/擦除等副作用，不宣称L03 configurable retry/state/resource验收。

旧36 lifecycle assertions/helper/fixtures BYTE保留，2个旧time-test literals只补identity
字段，原assertions/timefunction不改。prod除failureidentity/skip/record外、NativeSource/
guards/postpubquery/serial/watch/permits/manual/HTTP/auth/record/Core/public/access/deps
BYTE保持。sameID-host reuse/daemonrestart/未观察变化仍无新identity证据；失败map跨UID
retention仍无界，不把devicegate4096或source-ownedDrop当RSS/native task bound。
knownchange retry修正待pre-1.0 review/scoped device evidence。

control34585 terminal101 **0PASS/4FAIL**；post63939 terminal0 **40PASS**，
再增safeDebug privatehost fixture后冻结41。冻结 **79705 terminal0/all14 gates PASS**，
source/result audit terminal0。实际 **2550Rust/13Python**、CLI945/Core1443/public52、
0FAIL/2既有ignored；41/min41/tunnel82+1ignored/WebInspector192/access29/Core102/
minCLIalltargets/stack5/strict/fmt/tooling22/buildCLI95/full PASS。66features/4groups/
MSRV1.80.1/7+7docs/3snippets/CABI/3.12wheel-sdist/tooling67/help118/raw38/host5/CLI95/
backup3 PASS；**22device NOT_RUN**，Chromium/installed3.9/3.13 NOT_RERUN；
source一直冻结到authoritative completion，观察timeout不算exit。

patch SHA256 `b2479b78b7f65d429c8b626534bfe528f72b6f34a2f3171a9c52e77e6861fdeb`；
actual debug ios SHA256 `a6679568cd1d2b990d35c1d5afa2d6035521a3945d6f67d5fc15d7c03bf5f95f`；
commit=frozen，ignored evidence `target/validation/tunnel-backoff-2026-10-06/`，
reference HEAD/status不变。ActualCLIState independentknownID/pairedhost/route/IO/time
观测重试count/sourcecalls/ownedrelease，debug原secret无泄露；不是Core锁microtest/
physicaldevice/ACK/nativejoin/RSS/performance证据。

L02/第四类/goal active，9complete/10notstarted/12inprogressconditions/all31/全部子项/
历史/118capability正文maturity保留。接着failuremap/othermetadata bounds、afterquery/
reuse/restart/manualepochs/nativeLive/worker-serverjoins/discovery-wholeScan，L03/L04/
Q04及其他L01和第五至九类。无subagent/push/publication/historyrewrite/referenceedit/
设备或Apple服务操作。

### 大类 4：L02 known local liveness before admission/publication（2026-10-06）

受测实现 `7a77000cdbd0a8e6d2fbaaf6ac5bd630af47d6c5`，父
`d96cb4220eff078fec8e0b09d7a0c92ccd540527`。三源码103+/0-、四新测试；
[本地存活证据](validation/tunnel-liveness-2026-10-06.md) 与 [manager 合同](tunnel.md)
记录范围。生产仅two checks/sixlines：expectedID/recordvalidation后query前，以及
tablelock/epoch/policy后insert前，再读原device.is_alive。knownfalse拒绝派生
localOperationCancelled/ownedDrop/nofailurebackoff，allroutes含Wifi无daemonquery。
不重建nativeIO；其余prod/SDKSource/guards/query/backoff/watch/serial/permits/manual/
HTTP/auth/federation/record/core/public/access/deps及原41assertions/helper BYTE不改。

READONLY验证原SDK kernel cancelSender.receiver_count()>0，userspace everyhandle
!is_finished；SDK实现不改。true不是physicalconnect/AppleACK/nativejoinedworker，
receiverclone可造成falsepositive，worker可在lastcheck后退出；清楚保留这些条件，
不拿privateFakeAliveflag替代Coreworker/真机证据。admission修正待pre-1.0 review。

control52217 terminal101 **0PASS/4FAIL**；post30358 terminal0 **45PASS**；
冻结 **79395 terminal0/all14 gates PASS**、source/result audit terminal0。实际
**2554Rust/13Python**、CLI949/Core1443/public52、0FAIL/2既有ignored；
45/min45/tunnel86+1ignored/WebInspector192/access29/Core102/minCLIalltargets/stack5/
strict/fmt/tools22/buildCLI95/full PASS。66features/4groups/MSRV1.80.1/7+7docs/3snippets/
CABI/3.12wheel-sdist/tooling67/help118/raw38/host5/CLI95/backup3 PASS；
**22device NOT_RUN**，Chromium/installed3.9/3.13 NOT_RERUN。
source持续冻结到authoritativecompletion，观察timeout不当exit。

patch SHA256 `06cd466d7036ffe0256130289678df299b72dde19d642a3be0ea12549f06396f`；
actual debug ios SHA256 `8e7d076d086635b42dcd26102c5c1a6c816834dde91ec223b63423da0d535b3e`；
commit=frozen，ignored evidence `target/validation/tunnel-liveness-2026-10-06/`，
reference HEAD/status不变。actualCLIstate beforequery/whilequery/whiletable/Wifi-local
观察result ownership/no lateinsert/permit release；仅本地observedknownfalse，非physical
freshness/nativejoins/RSS/resourcebenchmark。Kernelreceiver positive语义限制明确保留。

L02/第四类/goal active，9complete/10notstarted/12inprogressconditions/all31/全部子项/
历史/118capability正文maturity保留。接着lastcheck后变化/kernelreceiverfalsepositive/
reuse-restart-manualepochs/nativeworker-serverjoins、retainedmaps/metadata/discovery-wholeScan，
L03/L04/Q04及其他L01/第五至九类。无subagent/push/publication/historyrewrite/
referenceedit/设备或Apple服务操作。

### 大类 4：L02 shared discovery setup deadline/epoch cancellation（2026-10-06）

受测实现 `a47648442d5b7707ad41beeb58a6ab1ad764ea9a`，父
`653a1a3021f4e429a1263f9aea997e6ea2f63e00`。四源码268+/9-、九新测试；
[发现期限证据](validation/tunnel-discovery-2026-10-06.md) 与 [manager 合同](tunnel.md)
记录范围。privateScanControl在queue之前捕获SCANepoch并borrow_and_update标记版本，
checkedInstant+existingtimeout(default15s)，drive共absolute deadline覆盖scanLock queue/
USBsnapshot/pairedsnapshot；stop-clear旧epoch返回None/quietOk，phasefuture/queue释放，
不当连接失败/ready。实际expire保留ORIGINALTokioElapsed/staticcontext；due-zero用
pending timer取得nativeElapsed而不poll realwork；overflow原inputdecision/invalid_input
在Source/mutatingpoll之前拒绝。此前这些wait无界；startup/quietcancel修正待release review。

边界：SETUP deadline在paired discovery返回结束，不涵盖wholeScan/candidatehandoff
CPU；原每candidate15s/4window/permits/nativeSource/IO/guard/query/liveness/backoff/
HTTP/Core/public/deps unchanged。sourceFutureDrop不是Core blocking pair-record workers
停止/join，仍缺更强parent-primary/worker/server/资源/platform/device证据。旧DeviceTicket/
weakgates/stop/clear/budgetbodies保留；原45assertions BYTE不改，fake仅phaseProbeDrop/
counters/pairedgate/per-requestcaptureddata。无SDKfakeconstructor/API/新protocol/pairing。

control79873 terminal101 **0PASS/7FAIL**；post58753 terminal0 **52PASS**；
再增queuedcancel/Scopeboundary+overflowmachineclass后冻结54。冻结 **32405 terminal0/
all14 gates PASS**、source/result audit terminal0。实际 **2563Rust/13Python**，
CLI958/Core1443/public52、0FAIL/2既有ignored；54/min54/tunnel95+1ignored/
WebInspector192/access29/Core102/minCLIalltargets/stack5/strict/fmt/tools22/buildCLI95/full
PASS。66features/4groups/MSRV1.80.1/7+7docs/3snippets/CABI/3.12wheel-sdist/tooling67/
help118/raw38/host5/CLI95/backup3 PASS；**22device NOT_RUN**，
Chromium/installed3.9/3.13 NOT_RERUN。source持续冻结到authoritativecompletion，
观察timeout不当exit，fakeFutureDrop非physical/nativejoin/RSS/ACK/baseline性能证据。

patch SHA256 `0afafe77afebd925b6afc42c93790d0729d56f65a93423cb70391f44815b3705`；
actual debug ios SHA256 `047970a9711ef1ee5d91f7ff8cc83af1ea0a74bb3841ba3106293f99d27ed969`；
commit=frozen，ignored evidence `target/validation/tunnel-discovery-2026-10-06/`，
reference HEAD/status不变。ActualCLIstate nativeElapsed不同于outertesttimeout、
sourceprobeactive/exited/ready/no IO与下一generation可用，quietstop旧assertions保留。

L02/第四类/goal active，9complete/10notstarted/12inprogressconditions/all31/全部子项/
历史/118capability正文maturity保留。接着retainedmaps/metadata limits、wholeScan/CPU/
handoff/source-worker/serverjoins、lastcheckchanges/receiverfalsepositive/reuse/restart/
manualepochs，L03/L04/Q04及其他L01/第五至九类。无subagent/push/publication/
historyrewrite/referenceedit/设备或Apple服务操作。

### 大类 4：L02 failure retention and capacity（2026-10-06）

受测实现 `26b89951b52dd690bcc0e994541007b5426fba93`，父
`405bb0d89b2b5e428cceae8b23924db9dff464af`，四文件474+/3-，十新测试。
[保留与容量证据](validation/tunnel-retention-2026-10-06.md) 记录实际 manager：
扫描按有效快照清理未授权、观测缺失与已知 identity 变化的旧失败；paired discovery
失败仍是 UNKNOWN，保留旧 mobdev2 行。仍在场且过期的相同 identity 保留 counter
和30/60/120/240/300s退避。观测缺失不是物理 ACK；清理后重新出现可从初始退避开始。

4096 个 owned semaphore slots 同时覆盖 retained rows 与 in-flight reservations；
已有行重试复用许可，新 UID 在 native IO/共享 connect permit 前预留。满额返回
原 source-free resource_limit，不 poll 新连接或静默淘汰旧退避。成功/取消释放局部
reservation；记录失败在 write lock 内重查 Ticket，clear/stop 后排队旧结果不复活。
数量上限不覆盖字节/RSS、candidate vectors、元数据 CPU、原生线程/socket。

十测试覆盖12行后空快照、paired error/USB缺失、实际 protected HTTP policy reload、
known ID改变先清理、same expired count2、4095 retained+held A占满/B无native IO/
failed A为第4096行、满额已有A重试、caller Drop、clear/排队写竞态、成功发布释放。
容量预填走实际私有 reserve/record，并非4095次原生建连；fake Drop 非 native join。
原54测试断言/nativeSource/lifecycle/Core/HTTP/public/trust/deps 保留，仅三旧结构体
fixture 增 slot:None；生产行持有许可。无新协议、配对、公共 API 或依赖。

control43601 terminal101 **1PASS/4FAIL**；恢复 after.log 为59PASS，但 parent
handle/exit不可取得，仅记录日志。expanded9659 terminal101 **61PASS/2FAIL**：
填充记录后首次 poll 因 Tokio 协作调度让步尚未到 Source，fixture 改为在外层1s
预算内等实际 entry 再检查，生产实现不变。最终 **94282 terminal0/all14 gates PASS**，
源码审查 terminal0。实际 **2573Rust/13Python**，CLI968/Core1443/public52、
0FAIL/2既有ignored；64/min64/tunnel105+1ignored/WebInspector192/access29/Core102/
minCLIalltargets/stack5/strict/fmt/tools22/actualCLI95/full PASS。
第11节 Rust/Python 拆分使用项目 .venv；66features/4groups/MSRV1.80.1/7+7docs/
3snippets/C ABI/3.12wheel-sdist build-install-import/tooling67/help118/raw38/host5/
CLI95/backup3 PASS。**22device NOT_RUN**；Chromium/installed3.9/3.13 NOT_RERUN。

patch SHA256 `324521a21287418fc406e9aeb1445196a16c47f8fa9dd2b3dd7ecdfa22ed8cfd`；
actual debug ios SHA256 `ef6ad9d64e17f6468043c31a940a64ac8927dadf252343726d7f11aaaf856bd0`。
commit=frozen，ignored evidence `target/validation/tunnel-retention-2026-10-06/`，
reference HEAD/status未变。cache/admission timing修正待pre-1.0 release review。

L02/第四类/总体 goal active；9完成/10未开始/12进行中或待条件，全部31主任务、
子任务、历史及118 capability正文/maturity保留。继续其他 metadata/字节上限、
wholeScan/CPU/handoff/native-worker/server joins、last-check/reuse/restart/manual epoch、
其他 L01、L03–L04/Q04 与第五至九类。未启用子代理、推送、发布、改写历史、
修改参考 checkout 或操作真实设备/凭据/Apple 服务。

### 大类 4：L01/L02 tunnel server shutdown ownership（2026-10-06）

受测实现 `5120fa90e06a92f4734603382b98779b69f1ca38`，父
`b052752bed4c1b1d26d3fa2f70a27aabc7218b33`，五文件498+/34-、八新测试。
[服务器退出证据](validation/tunnel-server-2026-10-06.md) 覆盖实际 listener/router/
manager：private watch bool 保留 shutdown，迟到等待者不丢退出；parent Box 拥有
scanner 与 candidate futures，取代 detached monitor。启动完成后一个 pinned
Tokio Ctrl-C 与 HTTP shutdown/server outcome 统一触发退出；先 drop scanner，
再用独立默认2s共享 clear table 与 Axum graceful drain。正常主错误/cleanup Elapsed
保留，dual failure 持有两者/source 指向 primary；BrokenPipe仍为Disconnected。

Caller Drop 发送退出/epoch cancellation，仅 try-clear 未锁表，不能 async join。
Axum0.7.9 dependency 源码只读确认 HTTP/signal delegate tasks：排空超时/drop 不会
强制 join/abort 已委托 handler/upgrades。held-table/HTTP30ms fixture 明确释放剩余
锁/请求并观察响应EOF，不把 fixture收尾冒充产品原生 cleanup。configuration/startup
total、late admission/last-check races、Core workers/native joins、generic callerDrop
primary reporting 与平台/设备条件继续。启动到 listener/router/info 的原主体、原64
生命周期断言、nativeSource/controller/backoff-retention/Core/HTTP/auth/record/trust/
deps 不改；旧method fixture只适配 private waiter，两行输入不改断言。

八新测试：迟到 shutdown；preclosed 无Source；实际 HTTP POST shutdown释放 pending
scan A/owned B/listener/permit，固定signal进入一次；caller futureDrop拥有scanner；
signal typed IO primary保留/清理；heldtable cleanup nativeElapsed加原BrokenPipe；
graceful HTTP drain期限；Linux真实子进程SIGINT在实际Tokio signal poll/source entry
之后发送，仅本测试子进程，退出runtime并验证假设备资源。不是实际设备/CLI startup
或其他平台信号证据，无新协议/配对/SDK假构造器。

control18746 terminal101 **0PASS/1FAIL**；34085 terminal101 E0599/no tests，
fixture响应改bytes+serde，无依赖；55083 terminal101 **71PASS/1FAIL**（分类误期望
Io，原BrokenPipe是Disconnected）；first frozen71101 terminal101 **71PASS/1FAIL**
（raw Anyhow Debug含保留的IO文本），新fixture改查safe wrapper与实际CLI诊断边界。
失败冻结源码hash/log保留。最终 **86684 terminal0/all14 gates PASS**，源码审查0；
实际 **2581Rust/13Python**，CLI976/Core1443/public52、0FAIL/2既有ignored，
72/min72/tunnel113+1ignored/WebInspector192/access29/Core102/minCLIalltargets/
stack5/strict/fmt/tools22/actualCLI95/full PASS。第11节项目 .venv 与 Rust/Python
拆分，66features/4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist
build-install-import/tooling67/help118/raw38/host5/CLI95/backup3 PASS。
**22device NOT_RUN**；Chromium/installed3.9/3.13 NOT_RERUN，maturity不升级。

patch SHA256 `d067f8f9ae5cf0a92b48926042d4281683ae0ca2989077e01c0e1ea7f40c2fc5`；
actual debug ios SHA256 `3823aedc1950ad79587f8701ed6ceede9f1b9a1320688d6d5f79e0f69e79980c`。
commit=frozen，ignored `target/validation/tunnel-server-2026-10-06/`，refs HEAD/status
未变。退出/时序修正待release review。L01/L02/第四类/goal保持active，9完成/10未开始/
12进行中或待条件，全部31主任务/子项/历史及118能力正文保留；其他 L01/L02、L03–L04、
Q04协议fuzz/高成本专项与第五至九类继续。无subagent/push/publication/historyrewrite/
referenceedit/真实设备/凭据/Apple服务操作。

### 大类 4：Q04 explicit high-cost PBKDF2 entry（2026-10-06）

受测实现 `de74436beeb302fd1756dd10d7f6a9f324344de9`，父
`74b47ca06f5dc149287217a18e747f77b9b2d011`，三脚本231+/1-。
[高成本证据](validation/pbkdf2-2026-10-06.md) 与 [构建入口](build.md) 记录
显式 high-cost scope，使用项目 .venv；普通 full 不加入10M向量，原ignore理由保留。
runner沿用既有Unix-owned process-group capture，默认900s覆盖Cargo编译+执行，
接受有限(0,3600]预算，合计输出2MiB。仅Core/lib/release/no-default/
backup2-manifest/locked精确选中既有public pyiosbackup vector，不连接设备。

通过需要精确目标行、唯一1PASS/0FAIL/0IGNORE/0MEASURED summary、唯一实际Core
test artifact及target内路径/sha；非零/零过滤/错误目标/重复summary/缺失产物/
partial/overflow/timeout/stop/未收尾group不能PASS。原始有界stdout/stderr保存于
新建ignored owned目录，sha与九个source文件在结束后再核对。六新工具测试含实际
validate dispatcher的PATH合成Cargo/真实文件/日志observer；该合成用例不当crypto
通过证据。没有Core/crypto fixture/Cargo/SDK/protocol/capture原实现变更。

首实际19040 terminal0：向量1PASS，63.952s含初次编译；历史原九sourceSHA保留。
最终冻结 **52988 terminal0/all5 gates PASS**：syntax、项目.venv工具73、
Core最小backup2-manifest/alltargets、实际high-cost、第11节full拆分均通过。
最新实际向量 **1PASS/0FAIL/0IGNORE/320filtered**，1.102s含缓存后的Cargo执行，
进程组cleanup_complete，无timeout/overflow/partial，产物与原始日志哈希审查0。
该耗时是单一host工作负载记录，不是L04性能基线或硬件吞吐承诺。
普通 **2581Rust/13Python**，CLI976/Core1443/public52、0FAIL/2既有ignored；
tooling73/help118/raw38/host5/CLI95/backup3、66features/4groups/MSRV1.80.1/
7+7docs/3snippets/C ABI/3.12wheel-sdist build-install-import PASS。
**22device NOT_RUN**；Chromium/installed3.9/3.13 NOT_RERUN，maturity不变。

patch SHA256 `378e6c40ff887bf7a4f933d8be41e54d74c6e5b783a75de1d65af0195f5898f9`；
actual ios SHA256 `3823aedc1950ad79587f8701ed6ceede9f1b9a1320688d6d5f79e0f69e79980c`；
high-cost test artifact SHA256 `889506219808b1c2c960d993366b70d2e68c8cf0c52bd9cffb6535502cd2016b`。
commit匹配冻结脚本，actual报告保留其运行时source_revision父提交及九sourceSHA；
ignored evidence `target/validation/pbkdf2-2026-10-06/`，references HEAD/status未变。

Q04进入进行中，总览 **9完成/9未开始/13进行中或待条件**，不以高成本子项完成
宣称协议fuzz通过。四类parser入口/完全合成种子/截断长度深度/UID压缩、预算内
panic/CPU/失控分配及decoded peak仍待完成；macOS证据/Windows通用Cargo子进程树
监督缺条件。普通跨平台full的scope不因新增显式入口而停用。全部31主任务/子项/
历史/118能力正文与L01/L02/L03–L04/第五至九类保留，goal active；无subagent/
push/publication/historyrewrite/referenceedit/真实设备/凭据/Apple操作。

### 用户范围修订与新会话交接：排除 Q04（2026-10-06）

用户先要求「把这个任务排除，然后给我个新的 prompt」，确认当前所指为
Q04「协议 fuzz 与高成本专项检查」，随后明确暂停本会话工程目标，并确认
「回退这个网络协议任务的改动，然后给我新 prompt」。本会话 goal 已为 paused；
本次仅整理范围、撤回实验和交接，不继续工程实现或启动验证构建。

有效新目标为原31项中除 Q04 外的30项及其全部子任务，9完成/9未开始/
12进行中或待条件；Q04全部子项排除且不计完成。九大类顺序保留，第四类继续
L01–L04，再按依赖推进第五至九类。既有Q04高成本入口
`de74436beeb302fd1756dd10d7f6a9f324344de9`/
`d025b44e0ba25fa1d6f9658203712cd067d55ed4` 及历史证据保留，不改写历史。

本轮未提交的四个 Q04 实验文件已完整归档到 ignored
`target/validation/excluded-q04-2026-10-06/`，含 source copies、
`wip.patch`、`manifest.json` 和逐文件SHA256；归档patch SHA256：
`5930c66b2fa555c34779a12049f839cc7f41f229eb448151c2b236b3bb0081a4`。
两受控源文件恢复至受测基线 d025b44，新增 resource.rs/resource_tests.rs
移出工作区；逐字节与baseline核对，不重置其他工作区变更、不提交实验代码。

真实诊断历史保留：编译28760退出0，受限独立测试子进程退出101，1PASS/1FAIL；
525字节合成归档展开1,048,575节点，667ms、峰值93,688KiB RSS。
83824编译退出101/E0603，无测试；改用Serde后的最新版只完成fmt13915退出0，
未编译/测试，现已归档撤回。该已知资源风险未被修复，不能把撤回或排除称为修复。
相关风险若阻碍 F06/D02 等任务原有验收，仍保留具体待条件，不自动豁免安全门禁。

恢复后的源码与最后主机完整验收基线 d025b44 一致：2581Rust/13Python、
73工具测试、0FAIL/2既有ignored；PBKDF2专项1PASS为既有历史证据。
本次仅作源码字节核对、范围文档与交接审查，未重新启动构建/测试。
22device NOT_RUN，平台/凭据/Apple条件保留；所有118能力正文/maturity及原任务、
子任务、交付历史保留。未推送/发布/改写历史、修改参考checkout或启用子代理。

### 新30项目标恢复与 L01/L02 shutdown admission（2026-10-06）

新会话先 get_goal 返回无目标，已 create_goal：原31项排除 Q04 后的30个主任务及
全部子任务，不设预算、不恢复旧31项目标、不启用子代理。完整读取 AGENTS.MD、
本计划7545行与指定交接；实际分支/HEAD为预期 `feat/device-service-updates-20260927` /
`25d3667ab41ac3fe30a5028128dd58269eeecad8`，工作区干净，无运行中的项目验证进程。
基线 d025b44 到恢复HEAD只有两个范围文档差异；归档patch/逐文件hash、两个撤回
源文件字节及两个实验文件不存在均核对，恢复阶段未构建测试。只读refs保持原HEAD
与既有uv.lock/go.mod改动；ignored恢复证据 `target/validation/resume-excluding-q04-2026-10-06/`。

受测实现 `0d025dd3404082fb76adab20758b2e278b2d9464`，父为恢复HEAD；四个CLI
文件334+/8-、九新测试。[关闭后接入证据](validation/tunnel-shutdown-admission-2026-10-06.md)
记录实际漏洞触发：已委托的 HTTP start 请求在 shutdown/清表后才完成 JSON body，
原coordinator可取得新epoch并重新建连。现 private lifecycle 区分可重用clear与不可
重开的shutdown终态；notify在async cleanup前close，旧tickets取消、新manual/scans
拒绝。普通clear可重新开始，server restart需新state。coordinator ticket在锁等待前
捕获原clear epoch，不会在等待后取得替代epoch；锁等待本身的total/cancellation仍待。

实际旧生产负控制85563退出101：1PASS/5FAIL；修复后32475退出0：原六项6PASS。
再加discovery/callerDrop/coordinator wait共九项。冻结11055权威退出0，八门禁全通过：
new9/min9、tunnel122/1既有ignored、minCLIalltargets、原stack5、strictCLIClippy、fmt、
第11节完整拆分。**2590Rust/13Python、0FAIL/2既有ignored**，CLI985/Core1443/public52；
66features/4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist build-install-import/
tooling73/help118/raw38/host5/CLI95/backup3 PASS。另项目.venv工具26755退出0：73PASS。
**22device NOT_RUN**；Chromium/installed3.9/3.13 NOT_RERUN，不升级maturity。

actual server/router的延迟body在观察清表后释放，502/cancelled、A零source contact/
无迟到publication、graceful drain/监听关闭；pending native-future/query/serial/discovery
取消及owneddevice/permit释放有主机证据。coordinator wait fixture明确先释放其持锁，
不宣称该锁等待立即取消。原断言/serverowner/NativeSource/Core/SDK/bindings/deps/访问
与wire保持，Q04源文件仍为恢复基线；无协议猜测、真实设备或Apple操作。

patch SHA256 `995001651bbd61e410eb00803cddd765ce99bb067cfac01b4fa3470bb9bd8c3f`；
actual ios SHA256 `a5d0be4a0f7246e098eaaa8397268ac7d48768bb9828b8e638a83bdd64a57fc4`。
commit patch与frozen完全相同；ignored `target/validation/tunnel-shutdown-admission-2026-10-06/`
保留日志/退出码/源码拷贝/hash与审查。首文档patch context不匹配未应用，重读后修正；
没有放宽断言/lint/条件。终态接入语义修正需pre-1.0 release review。

L01/L02/第四类/新30项目标 active，**9完成/9未开始/12进行中或待条件**。全部原
任务/子项/历史与118能力正文/maturity/既有证据保留，仅追加本host profile。继续
coordinator lock-wait total/cancellation、post-check竞态、startup/wholeScan/metadata/
nativeworker-serverjoins与其他 L01/L02，再 L03–L04、第五至九类及其他待验收条件。
不把本项当atomic全竞态保护、asyncDrop/nativejoin/physicalfreshness/ACK/性能基线。
Q04全部子项继续排除，UID已知资源风险未修复，其他任务原验收不豁免；无cacheclean/
subagent/referenceedit/push/publication/historyrewrite，源码冻结至验证真实退出再提交。

本项七文档收尾审查：六Markdown/353 local links、项目.venv capability inventory/
actualhelp118、whitespace与实现/二进制hash通过；118能力正文/maturity/所有旧evidence、
全部历史/子任务prefix保留，只有L01/L02当前行和末尾交付追加。文档期间源码未变。

### 大类 4：L01/L02 coordinator lock-wait cancellation（2026-10-06）

受测实现 `a4beec7fa01b605933b1966b79ef3ee738ae41fd`，父文档收尾
`8bd99de979495a7ea710925c072546015b0e2a80`；一个lifecycle文件40+/2-。
[协调器取消证据](validation/tunnel-coordinator-cancel-2026-10-06.md)覆盖先前准确
保留的锁等待不能立即取消条件：直接owned biased select同时等待clear/shutdown与
实际coordinator mutex，锁仍被持有时返回原OperationCancelled，不产生coordinator行。
原post-lock epoch检查/已有断言保持，普通clear之后fresh ticket可继续；无新task、
协议/public API/权限/NativeSource/Core/SDK/bindings/deps变动。

一新测试有clear与terminal两runtime路径；两者都在持锁期间观察typed取消及空表，
之后释放锁，clear分支再实际新建ticket。负控制35560退出101：0PASS/1FAIL，首次clear
等待外层1s超时，terminal路径该失败运行未到达，不能算两个负控制。冻结14142权威
退出0，八门禁通过：new1/min1（两路径）、tunnel123/1既有ignored、minCLIalltargets/
原stack5/strictCLIClippy/fmt/第11节full。**2591Rust/13Python，CLI986/Core1443/public52，
0FAIL/2既有ignored**；66features/4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/
tooling73/help118/raw38/host5/CLI95/backup3 PASS；另项目.venv工具90657退出0，73PASS。
**22device NOT_RUN**；Chromium/installed3.9/3.13 NOT_RERUN，不升级maturity。

patch SHA256 `0573916f2586d6509a1ce2ecf749670bf994df9bd3349a8ff3bb9bd44692a8d5`；
actual ios SHA256 `5ea60df57061c294c6b8252baf28376b02de6f8491365ceef054faa502284232`。
commit=frozen，ignored `target/validation/tunnel-coordinator-cancel-2026-10-06/`；只读
refs HEAD/status及全部其他产品源码保持。源码冻结至真实退出再审查diff/提交，取消
时序改变需pre-1.0 review；持锁因果fixture不作为L04性能基线。

L01/L02/第四类/30项目标 active，9完成/9未开始/12进行中或待条件及所有原任务/
子项/历史/118能力正文maturity保持。继续coordinator acquisition deadline与manual
connection total覆盖、startup/wholeScan/metadata/post-check竞态/nativeworker-serverjoins、
其他L01/L02，再L03–L04、第五至九类及全部待条件项。无asyncDrop/nativejoin/physical/
Apple/资源/性能认证。Q04全部子项继续排除，已知UID风险未修复，其他验收不豁免；
无cacheclean/subagent/referenceedit/push/publication/historyrewrite/device操作。

本项七文档收尾审查：六Markdown/358 local links、项目.venv source/actualhelp118
inventory、whitespace及source/binary hash通过；全部旧任务/子项/history prefix、118能力
正文/maturity/既有evidence保持。只有L01/L02当前行、末尾交付与一项host证据追加，
文档期间产品源码不变。

### 30项目标继续、子代理授权与 L01/L02 manual deadline（2026-10-06）

用户指定继续会话 `01a11071-8467-7432-a816-7286a9a898eb`，开始前要求报告剩余任务。
核对路线图/工作区后已报告：有效30项，9完成、**21未完成**（9未开始、12进行中或
待验收/条件），Q04全部子项排除且不计完成，22真机项单列 NOT_RUN。当前HEAD为
前会话实际交付 `1782d1f4f487d3ba05c6d6ee0ddc3d8744bfd8f9`，工作区干净、无项目
构建/验证进程。get_goal返回空，已 create_goal：完整30项及全部子任务、不设预算。
用户随后明确允许子代理，要求避免同时编译；新授权覆盖旧交接禁止子代理要求。
两名范围审查代理和一名实现审查代理只读工作，不编译/测试/编辑/提交；主代理串行
执行全部Rust门禁，验证运行期间冻结源码。扫描内部锁、终态publication竞态、
Userspace原生构造失败任务泄漏与 L03设计已保存至ignored审查资料，尚不算修复。

受测实现 `d3ed1c33c14b2dcfdf83ef6c678cc1f451813176`，父为上述HEAD；四个CLI
文件427+/41-、13新测试。[manual建连总期限证据](validation/tunnel-manual-deadline-2026-10-06.md)
覆盖coordinator获取此前不计入15s的问题：先捕获checked绝对期限，admission与原
serial/cache/permit/connect/existing-trust fallback/attachment query/publication共用。
原生Elapsed、IO及clear/shutdown取消保持；overflow在Source前InvalidInput，zero
不poll Source。biased原生expiry与nominal work-poll guard阻止到期后的下一次poll，
迟到coordinator/permit/native ready/paired reply/publication lock不能继续该操作。
callerDrop及超时释放直接拥有的future/假设备/许可，不冒充原生worker join。

第一版八项旧ensure_tunnel负控制81949权威退出101：2PASS/6FAIL；zero失败仅到第一
route，不能算所有routes负控制。恢复fix后55001退出0：8PASS。只读审查建议增加
permit/fallback/完成device等待publication锁/实际HTTP边界；五新增fixture与cumulative
时间余量完善后冻结98985权威退出0，**全部九道串行门禁PASS**：new13/min13、
tunnel136/1既有browserignored、minCLIalltargets、原stack5、strictCLIClippy、fmt、
第11节完整拆分与项目.venv工具73。**2604Rust/13Python，CLI999/Core1443/public52，
0FAIL/2既有ignored**；66features/4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12
wheel-sdist build-install-import/tooling73/help118/raw38/host5/CLI95/backup3 PASS。
**22device NOT_RUN**；Chromium/installed3.9/3.13 NOT_RERUN，maturity不升级。

13项主机fixture直接覆盖真实mutex持锁直至product expiry、跨admission/connection
累计、四zero transports、overflow、迟到解锁/成功/paired回退、clear/shutdown取消、
callerDrop、四permit持锁/迟到释放、paired发现+actualMobdev2共用期限、完成device
在publication锁迟到释放时Drop，以及实际HTTP502/timeout与脱敏。累计用例为1s
总预算/600ms持锁/650ms观察余量，是因果期限测试，不是L04性能或跨平台承诺。
原断言及ensure之外manager/NativeSource/Core/SDK/bindings/access/deps保持；Q04
源仍与撤回基线一致，无新协议、pairing、真实设备或Apple操作。

patch SHA256 `bfb31c4b522a1221b01a38efcd4679787fc6bc38e2c59c9693d5ab6cda21cae3`；
actual ios SHA256 `e7820bf0f7d375ef1edb283a5ca867d280eee475f8d5303a3d49b35b4548aef0`。
commit patch=frozen；全部tracked crate/script/Cargo与新测试hash在门禁间及退出后核对。
ignored `target/validation/tunnel-manual-deadline-2026-10-06/`保留源码副本、日志、实际
退出码、hash与parallel follow-up audit；refs HEAD/status保持。无cacheclean/push/
publication/historyrewrite/referenceedit/真实device或credential/Apple操作。

L01/L02/第四类/完整30项目标保持active，**9完成/21未完成**及全部原任务/子项/history/
118能力正文maturity/旧evidence保留。下一步继续scan内部锁期限与取消、startup/
wholeScan/metadata/handoff、shutdown/publication linearization、原生construction/
worker-serverjoins、physical freshness/reuse/restart/manual epoch及其他L01/L02，再
L03–L04、第五至九类和全部待条件项。本项不preempt单poll CPU，不认证原生join/
ACK/asyncDrop/资源/性能；direct tunnel start与scanned candidate timing未改。
期限/迟到结果时序修正待pre-1.0 release review。Q04全部子项继续排除，UID风险
未修复，其他任务原验收不豁免；不得按小项完成结束总体目标。

本项七文档收尾审查：六Markdown/364 local links、项目.venv capability source及
actualhelp118、whitespace、commit/frozen patch与source/binary hash通过。31原任务行
及全部history prefix、118能力正文maturity和既有evidence保持，仅当前L01/L02、
最新用户授权、末尾交付及本host证据更新；文档期间产品源码未变。

### 大类 4：L01/L02 scan setup internal waits（2026-10-06）

实现 `57d57999fddbe0234b502686b65b8500d7d50cfd`，父 `7ca396753cc89a38da980de9b0f558a7dd2eabdb`，四CLI文件726+/25-、
16新测试；[scan setup证据](validation/tunnel-scan-setup-2026-10-06.md)。一个子代理
实施、另一代理审查，均未运行Rust命令；主代理串行全部构建/测试，运行时冻结源码。
后续代理只读审查/ignored方案，不应用到产品源码。

原checked15s discovery clock扩至initial/post-USB table、USB coordinator和无授权
failed/ready锁；clear/shutdown在锁仍持有时结束等待。每次work poll校验nominal
deadline，迟到解锁不能prune/clear/markReady/插入Weak gate。USB source已经返回
的原生错误在同poll epoch变化后不被吞掉；ListError清理取消跳过旧写、返回primary，
cleanup expiry private wrapper分别保留IO/Elapsed，source指向primary且wrapper/CLI
human/JSON脱敏。任意raw source Debug并不自动脱敏。

旧refresh_once/ScanControl.drive与新scaffolding负控制74607权威退出101：0PASS/16FAIL；
组合helper负控制在首个失败子路径停止，不能算每个循环分支已分别负验。恢复fix后
冻结91653权威退出0，九串行门禁全部PASS：new16/min16、tunnel152/1既有ignored、
minCLIalltargets、原stack5、strictCLIClippy、fmt、第11节完整拆分、项目.venv工具73。
**2620Rust/13Python，CLI1015/Core1443/public52，0FAIL/2既有ignored**；66features/
4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/tooling73/help118/raw38/
host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN。
累计测试1s总预算/两300ms阶段/650ms观察余量，是因果期限证据，不是L04测量。
全部旧断言/其他product sources/Q04与只读refs HEAD/status保持。

patch SHA256 `b6e8621bde201da0271ae9c0ef9117109eccaeb759599719f070de8f6c6301c7`；actual ios SHA256
`f89cd7fe0fecc52b448df11b619ebf7865c09d73c3ef75bf5cfba7d62bd3ecbb`。commit patch=frozen；tracked crate/script/Cargo及新test hash在门禁间
及退出后核对。ignored `target/validation/tunnel-scan-setup-2026-10-06/`保留source、
control/gate logs、exit/hash/review。无cacheclean/push/publication/historyrewrite/
referenceedit/device/credential/Apple操作。

paired reply结束setup clock；candidate独立total与后续handoff/reconcile/finalready
保持原行为。single-poll CPU、shutdown/publication linearization、startup/wholeScan/
metadata、原生construction/worker-serverjoins、physical freshness/reuse/restart/manual
epochs、其他L01/L02、L03–L04和第五至九类及全部待条件项继续。有效30项及全部
子任务仍active，9完成/21未完成；Q04全部排除、UID风险未修复，其他原验收不豁免。
无asyncDrop/ACK/nativejoin/资源/性能认证；时序修正待pre-1.0 release review。

七文档收尾审查：六Markdown/369 local links、项目.venv capability source及actual
help118、whitespace、commit/frozen patch与source/binary hash通过。31原主任务行及
全部history prefix、118能力正文/maturity/既有evidence保持；仅当前L01/L02与本项
交付/evidence更新，文档期间产品源码未变。

### 大类 4：L01/L02 userspace native construction ownership（2026-10-06）

实现`19315169dda049d6ab1acd68414bf4976b147b47`，父`838caab433615ea56a1cc39ca8cf72683bbcea21`；Core一文件267+/3-，
一Linux test。[native construction证据](validation/userspace-construction-2026-10-06.md)。
原三个worker先于bind/local_addr、error时尚无owner，JoinHandle Drop会detach。现在
原IPv6/MTU验证后先完成listener/boundport，才channels/split/spawn；所有fallible
constructor步骤先于worker。成功四任务/public API/error/deps/所有旧断言保持。

独立exact current-thread child保持父limit，用owned /dev/null到真实EMFILE；三次
public start校验TunnelError::Io aggregate/class/native source errno、alive baseline、
streamDrop1/zero polls/actual peerEOF/yield无任务。释放FD并检查restore后真实native
success四任务/reader实poll/ownerDrop observed abort completion+peerEOF。一秒观察
不是explicit join/ACK/L04认证；local_addr错误及RAII panic恢复只作结构审查。
旧body direct-child97374权威退出101：0PASS/1FAIL，first attempt genuine EMFILE后
alive3/baseline0/drop0/read0/write0，后续EOF/loops/success未执行。

冻结62936权威退出0，11串行门禁PASS：new parent/directchild/min1/1/1、Coretunnel46、
minCoreuserspace+CLIalltargets、原stack5、strictCoreClippy、fmt、第11节full、.venv
工具73。**2621Rust/13Python，CLI1015/Core1444/public52，0FAIL/2既有ignored**；
66features/4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/tools73/help118/
raw38/host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN。
子代理只写ignored方案/审查，主代理应用source及串行全部Rust工具，运行期间源码冻结。
一次审查脚本错误将spawned task body的await算作constructor等待，已纠正静态审查；
没有product/test失败或source改动。control/source metadata在runner开始后完善，
其baseline/hash一致；旧断言/其他product/Q04/ref HEAD/status保持。

patch SHA256`981028a87d668173a4b132c8d8c5ee5b8310ac067cacb96a0caa0c88051da923`；actual ios SHA256
`0e9a4b375298a50bff627caa920cc931412426c299bed209ce45cf999115b6a7`；commit patch=frozen。ignored
`target/validation/userspace-construction-2026-10-06/`保留proposal/source/control/
gate logs/exit/hash/review。无cacheclean/push/publication/historyrewrite/referenceedit/
device/credential/Apple操作。成功userspace/proxy/kernel/blocking/HTTP worker ownership/
joins、publication/failure metadata linearization、scan terminal capture、startup/
wholeScan/metadata、physical freshness/reuse/restart/manual epochs及其他L01/L02、
L03–L04/第五至九类/全部待条件继续。有效30项及全部子项active，9完成/21未完成；
Q04全部排除、UID风险未修复、其他原验收不豁免，无maturity/资源/性能/平台认证。

七文档收尾审查：六Markdown/374 local links、项目.venv capability source及actual
help118、whitespace、commit/frozen patch与source/binary hash通过。31原任务行和全部
history prefix、118能力正文/maturity/旧evidence保持；仅当前L01/L02、本交付/evidence
更新，文档期间产品源码不变。

### 大类 4：L01/L02 tunnel shutdown state changes（2026-10-07）

实现`7dc589bf4f26ead3597d991992f251d0ec29831e`，父`1ff3db029375adbb99cf73087724ba8e789f34bb`；六CLI文件769+/26-，
10新测试。[shutdown state证据](validation/tunnel-shutdown-fences-2026-10-07.md)。
publication及failure row原有is_current后仍有窗口，close/clear/stop可以已完成；现在
private同步Ticket::commit持generation/shutdown watch read guards至实际mutation结束，
stale跳过/native取消、有效mutation先完成writer才结束。保留前置epoch/access/alive、
失败identity/count/slot、fallback/格式及所有旧断言；noawait/no watch重入。scan原
preclosed/checkeddeadline后subscribe可能把已完成close标seen，新增capture后terminal
复查；ordinary clear仍复用，cfg(test)一次hook取出后释放Mutex再call。

实际manager第二次is_alive pause等待close完成再放行，publication在cleanup前取证，
worker/closing threads先回收再assert。固定Usbmux/Usb/Wifi/Mobdev2四route计数与Drop/
serial/permit正确；commit先赢再close/clear、stale action、guard writer顺序、scan
capture-close/clear/overflow priority/drive无poll、failure stale新增/更新及slot回收。
原callsite/capture加测试seam/helper负控制9611权威退出101：6PASS/4FAIL，直接观察
关闭后record、USB1、failure新增、stop后旧metadata改变；first Usbmux/stop失败后其他
oldroute/clear分支未执行。六PASS中保留新helper，不冒充原helper负验。

恢复fix后冻结68894权威退出0/all9串行门禁：new10/min10、tunnel162/1既有ignored、
minCLIalltargets、原stack5、strictCLIClippy、fmt、第11节full、.venv tools73。
**2631Rust/13Python，CLI1025/Core1444/public52，0FAIL/2既有ignored**；66features/
4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/tools73/help118/raw38/
host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN。
子代理draft ignored方案/只读review，usage中断后主代理完成merged实现/审查/所有门禁；
无subagent Rust工具，运行期间冻结全部source，旧断言/其他product/Q04/refs保持。

patch SHA256`5d2ebc562a6af6c82abf888ee21c3f35b70a943bc4320e11a0cd7b3133fbb91a`；actual ios SHA256
`31c07770a14b5ce988debc3e82048113500f47b459ffbdd8fdff62c2441de6c8`；commit patch=frozen。ignored
`target/validation/tunnel-shutdown-fences-2026-10-07/`保留source/control/gates/exit/
hash/review；original ignored proposals在旧publication目录。无cacheclean/push/
publication/historyrewrite/referenceedit/device/credential/Apple操作。

commit先赢的response仍可在close后结束；close可同步等待mutation，在独立2s server
cleanup clock开始之前，不认证hard shutdown total或单poll CPU抢占。policy/physical
last snapshots、samepoll fallback、其他scan写、startup/wholeScan/metadata/native
ownership/joins、IDreuse/restart/manual epoch、其他L01/L02、L03-L04/第五至九类/全部
待条件继续。现有proxy JoinSet及abort/EOF已交付，不能重复实现或等同whole-owner
explicit join。有效30项及全部子项active，9完成/21未完成；Q04全部排除、UID未修复、
其他原验收不豁免，无maturity/ACK/资源/性能/平台认证。

文档收尾：七文件379个本地链接、118能力实际help、whitespace、commit/frozen patch
与435产品及binary hash通过。31原任务行与全部history prefix、118能力正文/maturity/
旧evidence保持；仅当前L01/L02、本交付/evidence和review日期更新，文档期间源码不变。

### 大类 4：L01 packet forwarding reader ownership（2026-10-07）

实现`a455d6788168b61b3bae49fa0a996b3fa2b5824e`，父`0981ee2e0e8c0b7af352abf4c236d9c5d59ee78d`；Core两文件387+/4-、7新测试。
[reader ownership证据](validation/forward-reader-owner-2026-10-07.md)。原forward reader
裸JoinHandle在caller future drop时detach，半帧/idle read仍持stream；局部JoinSet
现在在drop/abort/unwind时请求abort，正常结尾abort_all仍返回原outcome，不等待join。
所有旧forward/framing/MTU/64queue规则与断言保持，active framing与terminal取消区分。

七case：真实duplex消费20字节header后drop、controlled TUN write、实际capacity1
write_all写1字节及drain后EOF、controlled flush写40字节、parent abort/nativeJoinError、
parent TUN panic/typedpayload、normal watch-close保持Ok。先记录release/EOF/task，
再关闭peer恢复到Drop1及runtimebaseline，最后assert。原body负控制40930退出101：
1PASS/6FAIL；六取消case均在回收后release断言失败，normal旧abort已有故PASS。
恢复fix后冻结96089退出0/all10串行门禁：new7/min7、Coretunnel53、minCore tunnel/
CLIalltargets、stack5、strictCoreClippy、fmt、第11节full、.venv tooling73。
**2638Rust/13Python，CLI1025/Core1451/public52，0FAIL/2既有ignored**；66features/
4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/tools73/help118/raw38/
host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN。
子代理只读审查实际source/fixture/Tokio，主代理独自修改和串行Rust工具，运行期间冻结。

patch SHA256`45cacb4cf65201e5b934c62ce92ab3acd3a71d1adf90b889e83453a5924f877d`，actual ios SHA256
`c7b9351f4c47680f731be9e33e8b782700479d91c6da818102ba548f79a2f2b5`；commit=frozen。ignored
`target/validation/forward-reader-owner-2026-10-07/`留source/control/gates/exits/hashes。
其他product/Q04/refs保持；无push/publication/historyrewrite/device/credential/Apple操作。
仅scheduled abort/eventual release；normal reader join、reader panic传播、cleanup
deadline、outer kernel/whole-native owner joins、单poll CPU/阻塞Drop、其他L01/L02/
L03-L04/第五至九类继续。30主任务全部子项active、9完成21未完成；Q04全部排除、
UID未修复、其他验收不豁免，无maturity/ACK/socket/RSS/长跑/性能/平台认证。

文档收尾：七文件384个本地链接、118能力实际help、whitespace、commit/frozen patch
和436产品及binary hash通过。31任务行及全部history、118能力正文/maturity/旧evidence
保持，仅当前L01与本交付/evidence更新；文档期间产品源码不变。

### 大类 4：L01 packet forwarding watch cancellation（2026-10-07）

实现`a4fc6aa6ccf455773009c3a1a38a9d0e8ca67958`，父`028647e3801d4f098fa81da22d3ee087c4856ee5`；Core两文件278+/27-、6新测试。
[watch cancellation证据](validation/forward-watch-shutdown-2026-10-07.md)。原主select仅
选branch时观察watch，selected TUN/stream write及flush无限await；现在每轮biased
outer watch包完整inner select+写入，单receiver，private ForwardStep将原结果带至
loop/reader abort_all。连续Ready每轮重查；同poll取得的IO不被cancel吞掉。
旧375行/11723bytes测试prefix、所有旧断言、framing/MTU/queue/reader owner保持。

六case：关闭sender下controlled TUN/实际capacity1 stream write/controlled flush、
保留sender的send(())/receiver1→0、首read同步关闭后只read1/write40、同poll关闭且
返回stdIO BrokenPipe/typedFault/Disconnected。测试先存production结果，再drop(work)
并恢复stream/task后assert，不靠后续caller drop冒充watch成功。原body控制71973
退出101：2PASS/4FAIL，三close及send等待超时；本次busy/nativeIO pass，原select
unbiased不声明旧busy总先cancel。错误whole-loop outer-only控制90513退出101：
0PASS/1FAIL，下个source read返回finite markerIO，安全终止且回收后assert。

首次冻结52321退出1：完整验收failure-path工具指出新增test-only ForwardingFault
派生Debug违反owned-error脱敏门禁；exact source/logs保留failed-run-52321。进程退出
后改固定无载荷manual Debug，原verifier7 PASS，六behavior断言及controls生产代码保持。
修正后冻结52489退出0/all10串行门禁：new6/min6、Coretunnel59、minCore tunnel/
CLIalltargets、stack5、strictCoreClippy、fmt、第11节full、.venv tools73。
**2644Rust/13Python，CLI1025/Core1457/public52，0FAIL/2既有ignored**；66features/
4groups/MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/tools73/help118/raw38/
host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN。
lifecycle_audit只读actualdiff/fixture/Tokio、reconnect_design只读下一L03；主代理独自
写source和串行所有Rust工具，运行期间冻结。patch SHA256`ad4fcdf98289f55115fc2996de42b6ae5d83075ffc10c916e8df1e6ef88896e3`，
actual ios SHA256`5026030b49bf6787ab84c7947843ab0833f253b9724b94034fa4fa080b104cf6`；commit=frozen。ignored
`target/validation/forward-watch-shutdown-2026-10-07/`留source/两controls/gates/
exit/hash/review；其他product/Q04/refs保持，无push/publish/device/credential/Apple操作。

取消允许部分write丢弃并终态close，不认证rollback/ACK/单pollCPU抢占；当前取得IO
保留，未取worker queue错误可在ready cancel时不取。normal reader join/panic传播/
cleanup deadline/outer kernel与whole-native owners、其他L01/L02、L03-L04/第五至九类
继续；30主任务全部子项active、9完成21未完成，Q04全部排除、UID未修复、其他验收
不豁免，无maturity/OS TUN/socket/RSS/长跑/性能/平台认证。

七份文档、389个本地链接及118能力定义/成熟度/旧evidence审查PASS；旧路线图history/
子项完整保留，正文只改L01当前行再append本记录。436 product hashes、actual binary、
refs HEAD/status与committed=frozen核对PASS；catalog对同binary无重建检查118 PASS。

### 大类 4：L01/L02 Core native tunnel owner / retryable shutdown（2026-10-07）

实现 `f00ff25395a8432087daf578c9dd23975b09be5a`，父 `a90d70f9436e39f7b4be2f350610315b68376ddd`；14文件3261+/62-、35新测试。
[native owner证据](validation/native-tunnel-owner-2026-10-07.md)。库创建kernel保留原typed parent JoinHandle与原reader
JoinSet，userspace保留4typed parents与proxy-client set。Join只在专属async mutex中
借原handle，取得Ready后同步保存native cause再清handle/减pending；cancel/timeout
不take或detach，保留Arc后可用新budget继续join。Parent panic时childguard只abort，
runtime仍持原set。FinalDrop取消fallback，不冒充async join；globaltable不跨nativewait。

checked absolute shutdown clock与stop在方法调用时开始，future未poll也terminal。
mutex/nativejoin同clock，zero stop+nativeElapsed，overflowInvalidInput不stop；
pre/postReady防迟到成功，不抢占singlepollCPU/Drop。10fixed cause slots+饱和omitted
observations，firstwork primary/native JoinError/Elapsed可inspect；immutable report
joins_complete需pending0且accountingvalid，不认证无错误/deadline/ACK/externaltask。
原pendingcount下溢/上溢不能默默成功。已finished foreignabort不改quiet；childprereap
保原cause，unfinished竞争abort不声称exclusive来源。直接same-ledger workreport用
private Arcidentity避免childcause重复提升；foreign/full/extra wrapper仍普通nativework。

CoreManager stop/remove信号共享clone，register仅不同oldArc stop，sameArc保活；retire
移交stoppedArc供caller持有/retry，无manager退役队列。rawforward正常abort+join reader，
unexpectedJoinError与primary并存，raw无独立cleanupdeadline。原API/fields/errors/
Cargo.lock/productiondeps保持，rootopaqueexports按tunnel/backend cfg；仅testTokio
rt-multi-thread支撑controlled阻塞Drop。状态/正常返回耗时/错误变化待pre-1.0 review。

New35=owner/accounting/identity21+userspace6+kernel6+public2；原inline manager/userspace/
forward tests、readerowner文件/publicconsumer断言保持。Actual Tokio/duplex/loopback
proxy、privatehostTunDevice+实际factory/forwarder；Condvar真实readerDrop阻塞与prefixEOF
cleanupgate是controlledfixture，不是OSKernelTun；suppliederrno13不证明真实permissionfail。
五control38711/21708/36220/3228/53494均terminal101、0PASS1FAIL；在assert前解gate/恢复。
前4早于identity及后续fixture，#5早于两testlint等价改写，不冒充全部againstfinalbytefreeze。

初Kerneltest缺multirt、public watchResult等式compile修正后，初publicgate遇磁盘满/LLVM/
linker失败；terminal后只删166可重建target/debug/incremental/ios_core-*目录，保存
source/证据/binary/refs/其他cache，无cargo clean。后续CARGO_INCREMENTAL=0。冻结
78491terminal101因bareuserspace无mdns既有Clippydeadcode；baseline worktree1446
同40lib/19test messages，originalminimumbehavior/check不减，额外strictintegration用
coredevice-base/kernel+mdns、仍-Dwarnings/workspacestrict。45196terminal101两新testlint；
exact failed freezes/logs分别存档，terminal后等价iflet/io::Error::other，无production/
断言变化。最终76373terminal0、全部14 gates重新串行，运行期间442producthash不变。

native220/userspacemin77/kernelmin80含new35/24/29，bareforward24/Coretunnel92，
minCore/CLIalltargets、stack5、strict3groups/fmt/full/.venvtools73 PASS；**2679Rust/
13Python，CLI1025/Core1490/public54，0FAIL/2既有ignored**。第11节66features/4groups/
MSRV1.80.1/7+7docs/3snippets/C ABI/3.12wheel-sdist/tools73/help118/raw38/host5/
CLI95/backup3 PASS；22device NOT_RUN，Chromium/installed3.9/3.13 NOT_RERUN。
Root独写source/独串行Rust，agents只ignored frozenproposal/readonlyreview。PatchSHA256
`031ba4f40065514b6579beded599824344ec43b54a15ef2d15d589d3c726cd26`，actualiosSHA256
`8d17a4f2d664af6bfba7db61d180eb2f2fceed4faa2e2db043a37a3729ddcaa3`；commit=frozen，refsHEAD/status不变。
Ignored `target/validation/native-owner-design-2026-10-07/`留controls/source/gates/
exits/hashes/baseline/cache/审查。无push/publish/historyrewrite/device/credential/Apple操作。

Actual activation在RSDawait前同步attachkernelparent，但Source返回前cancel仍整个owner
Dropfallback，CLI无retainretry。下一项实际CLI create/native取得至metadata/tableawait、
eviction/replacement/clear/policy/snapshot/error/server-heldtable/retainedstate与standalone
需要独立boundedretirement/nativejoins；genericHTTP/upgrades/blocking/startup/wholeScan/
metadata/physicalfreshness/reuse/restart/manualepochs/其他L01-L04/第五至九类继续。
有效30主项全部子项active、9完成21未完成；仅Q04所有children排除、UID未修复、其他
验收不豁免，无maturity/ACK/rollback/OS TUN/RSS/长跑/性能/平台认证。本profile不完成主项。

实现后10文档记录已核对551个local links、118能力正文/maturity/全部旧evidence、原31
任务行与全部history/subtasks；有效30仍9完成/9未开始/12进行中或待条件。442product
hashes/actualbinary/commitpatch/只读refs保持，capabilities按同一actualbinary复查PASS。
独立文档审查指出Core stop先remove释放表锁再signal、overflow不requeststop（不复活
旧stoppedowner）及baselineaudit实际session1446，均按source/JSON更正；主代理完成
计数与范围audit。子代理后续达到使用限额，已交付冻结资料保留，后续由主代理继续；
不是任务阻塞或验收完成。文档未重跑Rust；没有产品变化，原14串行gates证据保持。

### 大类 4：L01/L02 CLI native retirement / pre-RSD owner（2026-10-07）

实现 `6debaf9b6d78c10b4be1cfd55e2fbcbb69da8756`，父 `3bf026b29af8798e7600fc51aee5e9f290b7ffe9`；24source3671+/77-、39newtests。
[验收证据](validation/cli-tunnel-native-retirement-2026-10-07.md)。CLI 在Source poll前
预留capacity；Core四个additive with_owner API保原signature/options/fields/errors，
per-target guard在strategy transport前reserve、exact originalArc在initialRSD await前
attach，complete只移交owner不commit publication；原optional probe失败仍nonfatal。
RemotePairing同Source前target取消后的nativeowner独立退役，nexttarget须新reserve。

固定4096 combined reserved/inflight/unpublished/active/retired slots+opaque incarnation/
epoch，source handoff先于metadata/query/tableawait；所有ManagedTunnel Drop退役并
保原Arc/joins，不在table/watch/stdmutex下await join。明确stop/clear/replacement先释表，
其他retain可同步退役但不join。取消/期限保留原pendingjoins，新clock可retry；release仅
direct native shutdownOk或firstouter typed report valid-accounting joins_complete，
foreign nestedtrue/isAlive/finishedAbort/pendingcountalone不认证。Readonly status与
has_retained_outcomes捕获outertimer取消innerfuture前已取得nativeIO；fixedfirstexpiry/
firstnative history使同owner后来原fault不被旧Elapsed遮住，fresh attemptclock独立。

Server先seal/stopall后dropownedscanner，不依赖metadata表锁；onechecked independent2s
clock（先capture再shutdown sideeffects）内native与metadata/HTTP并行drain、Ready前保存
phasecause。Ok须两phase完成、无current/historyfault且closedempty；heldtable可在native
全join后elapsed，facts保留。metadata-only directElapsed及原signal/serverprimary保持。
Standalone实际source接owner-awareCore与returnedbody后2s native drain；callerDrop只
同步stop、保owner可retry，不async join，不认证startup/physicalCLI。无detached cleanup。

39new=14sender-only registry contracts（0native，不作cleanup证据）+5actualnative+13actualCLI
workers/privateMetadataSource+3actualCoreactivation+2status/public+2rawForward work/panic。
真实Tokio/duplexEOF/proxychild（TCPclosure EOF或reset）与CorelocalCDTunnel；controlled
Condvar Drop/panic/suppliedBrokenPipe非OS fault，capacity为1actualruntime+4095empty。
原inline manager/userspace/forward tests、readerowner/Cargo.lock byte保持；CLI旧asserts不改。
Controls85843 omitretirement/12785 omitpreRSDattach/5419 omitacquiredcause均terminal101/
0PASS1FAIL，恢复后450hash一致；#2故意无original retainedowner，仅fallback EOF/tasks，
不冒充原join。旧70385编译、97136 TCP EOF假设、75313新rawfixture编译、62685新lint失败
exactsource/log保存，terminal后修复，不在active validation期间编辑。

最终73569terminal0/all16串行，450product before/afterhash、16loghash、actualbinary/refs
一致。CLI194/1ignored/Core-native227/minuserspace83/minkernel83/bareforward26/Coretunnel95，
Corebare/tunnel/CLIminalltargets、oldstack5、strictintegration/workspace/fmt/full/tools73 PASS。
**2718Rust/13Python，CLI1057/Core1496/public55，0FAIL/2既有ignored**；MSRV1.80.1/
66features/4groups/docs7+7/snippets3/C ABI/3.12 rebuiltwheel-sdist/help118/raw38/host5/
CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN；maturity不升。
PatchSHA256 `11582477b507c70a75dced8f9ccfbdb81dbbbc800f0d55b1099403e377b287a0`，actualiosSHA256
`376add85e6cfec4f2b1b11d213021c03ce3bee6d1a454c87a66682fde51d4193`；commit=frozen，refsHEAD/status不变，CARGO_INCREMENTAL=0，
此slice无cache删除/cargo clean。Root独写source/独串行Rust；agents v1 frozenproposal/
readonlyreview保留，后达quota、未假定后续交付，不是goal阻塞。Ignored证据在
`target/validation/cli-native-retirement-2026-10-07/`。无push/publish/historyrewrite/
device/credential/Apple/elevatedTUN。Timing/error migration待pre-1.0 review。

有效30主项全部children仍active，**9完成/21未完成（9未开始/12进行中或待条件）**；只
Q04全部子项排除，UID未修复，其他验收不豁免。GenericHTTP/upgrades/blocking/startup/
wholeScan/metadata/physicalfreshness/reuse/restart/manualepochs及其他L01-L04、第五至九类/
全部待条件继续，无ACK/rollback/retry/OS TUN/RSS/peak/性能/长跑/平台或真机认证。
所有原31task行、全部history/subtasks、118能力正文/maturity/旧evidence保持；本profile
不完成主项/第四类/总体goal。实现之后单独文档提交、source/binary不变，goal继续。

文档收尾核对1028个local Markdown links、118capability正文/maturity/全部旧evidence、
原31个current task rows与7965行原history prefix（只L01/L02当前行更新）；有效30仍
9完成/9未开始/12进行中或待条件。450product hashes、actualbinary/commitpatch与
只读refs保持，capabilities针对同一已验证binary复查PASS。仅文档变化，未重跑Rust；
16串行gates的真实源码证据保持，不冒充其他平台/真机验收，goal继续active。

### 大类 4：L01/L02 manual same-poll deadline crossing（2026-10-07）

实现 `a963679ab0f853b1a38bfcee9ac59471d4920641`，父 `9b2e355f4bd2237afd130beb0b0a45634aa4ea48`；4 source files357+/6-、7newtests。
[验收证据](validation/tunnel-manual-poll-crossing-2026-10-07.md)。同originalchecked15s
clock在Source admission、completed native handoff、metadata后/query前、publication前后
同步复查。一个Source/metadata poll跨clock不再继续query/publication；default fallback
保已取得原IO并在due时不再contactpaired/transport。Private payloadfree decision回到
independentnative timer取得actualElapsed，lateReadyOk拒绝、原nativeIO不遮盖。

Real tableinsert同poll跨clock时，在table+epoch fences下remove未观察newrow、还原
displaced originalrow及exactliveArc；释fences后latelease退役stop并保原nativejoins。
这是本地表还原，不是device事务rollback/ACK/replay认证。CPU/metadata/Drop仍cooperative。
Automaticcandidate继续old policy；wholeScan/genericHTTP/upgrades/blocking/startup/metadata
字节预算/physicalfreshness/reuse/restart/manualepochs及其他原L01-L04条件仍需完成。

7new=5actualuserspace nativeworkers/DuplexStream/privateSource metadata+2real private
wrapper；controlled200msclock/250mssynchronouspoll不是L04性能。原13nativeCLI testfn
byteidentical，其他oldtests/Core/public/fields/errorenums/options/wire/deps/features/lock保持。
Baseline98426 terminal101/1PASS4FAIL，3actual cases先join/EOF/tasks恢复后regression；
firstfixed51593 fivePASS，complete38308 sevenPASS。Control81695禁postinsertcheck/rollback，
terminal101/0PASS1FAIL，两个originalnative joins/status/EOF/taskbaseline先恢复再fail；
只计划file改变，final前恢复全部451producthash。Privatecfgtestcallback只controlleddelay，
不持callbackmutex、无生产hook/SDK fake ctor。Exactfailedsource/log/hash保留。

Runner70521terminal0/all7串行：CLI201/1ignored/minCLIalltargets/oldstack5/strictworkspace/
fmt/full/.venvtools73 PASS。**2725Rust/13Python，CLI1064/Core1496/public55，0FAIL/
2existingignored**；MSRV1.80.1/66features/4groups/docs7+7/snippets3/C ABI/3.12 rebuilt
wheel-sdist/help118/raw38/host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed
3.9/3.13 NOT_RERUN，maturity不升。PatchSHA256 `35bd3891b6a7c152c1ffc1efbb493ca168dc8d62f8b1af988022cbd248f0c42b`，actualdebugios
SHA256 `3e06a80ec5fd666eee4615c7060bfb9219591778d1565d2aef1ef97f5964fc9b`；commit=frozen451hash，referenceHEAD/status不变。
Root独串行Rust/独edit，无activevalidation期间source/doc edits；agents quota无新交付。
CARGO_INCREMENTAL=0，无cache删除/push/publish/historyrewrite/device/credentials/Apple/
elevatedTUN。Ignored证据 `target/validation/deadline-crossing-2026-10-07/`；impl后独立docs。

有效30主项及全部children仍active，9完成/21未完成（9未开始/12进行中或待条件），只
Q04全部子项排除，UID未修复，其他原验收不豁免。全部old31task行/history/118capabilities
body/maturity/oldevidence保持；不完成主项/第四类/总体goal，后续第四类及第五至九类继续。

### 大类 4：L01/L02 automatic candidate checked deadline（2026-10-07）

实现 `c92503ded8a60515a272f4afb1dba99085b130dc`，父 `034d932148e329ed1564023526b9ebe21b948d0f`；4 source files214+/28-、7newtests。
[验收证据](validation/tunnel-automatic-deadline-2026-10-07.md)。Each admitted candidate
serial/cache/backoff/permit/Source及nativehandoff/metadata/query/publication同一个checked
absolute15s clock，原fourwindow/admission policy保持；due同poll Source/metadata不能继续
query/publish、realinsert未观察row还原，释table+epochfences后latelease保exactArc/joins退役。
Scannedcontext actualElapsed/原acquiredIO分类保留，manualcontext不变，无额外asyncwrapper。
Zero/overflow privatebudget不pollSource/noowner/failureentry、typedTimeout/InvalidInput。

7new=3actualnative workflow+2zero/overflow admission（修后0native）+2actualprivate scanned
wrapper类别；controlled200ms/250ms单poll非L04。旧manual7testfn byte同、其他oldtests/Core/
public/fields/errors/options/wire/deps/features/lock/UID保持。Wrong-old test-only baseline
89814terminal101/0PASS5FAIL，5cases实际nativejoins/status/streamEOF/taskbaseline恢复先于
regression；修后28534 sevenPASS。452productbefore/afterhash，exactbaseline/finalsource/
logs/source-review/frozenpatch保存；parentmanualcontrol仅历史，未冒充automatic新control。

Final52757terminal0/all7串行，CLI208/1ignored/minCLIalltargets/oldstack5/strictworkspace/
fmt/full/.venvtools73 PASS；**2732Rust/13Python，CLI1071/Core1496/public55，0FAIL/
2既有ignored**。MSRV1.80.1/66features/4groups/docs7+7/snippets3/C ABI/3.12 rebuilt
wheel-sdist/help118/raw38/host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed
3.9/3.13 NOT_RERUN；maturity不升。PatchSHA256 `dd242f5a526c23ec2a384552370be54e6e90230f6afd1b1ebe3ecb4e093549ab`；actualiosSHA256
`4c9f6ba765aa9d5b69a9a252513d70b271e95d5d9e8eaca587c7a095a21fdbca`。Commit=frozen452hash；referenceHEAD/status不变，CARGO_INCREMENTAL=0，
no cachedelete/push/publish/historyrewrite/device/credentials/Apple/elevatedTUN。
Root独serialRust/独applysource、agents readonlyreview、activevalidation无source/doc edits；
ignored证据 `target/validation/automatic-deadline-crossing-2026-10-07/`；impl后独立docs。

这仍是per-candidate total；queue未admit时未开始clock，failure record写入仍out-of-clock
可等锁。WholeScan paired之后handoff/reconciliation/readiness/failure-bookkeeping总期与
metadata bytes/HTTP/upgrades/blocking/startup/physicalfreshness/reuse/restart/manualepoch等
原条件继续；Source内部同步CPU/Drop不preempt。有效30主项及全部children保持active，
9完成/21未完成（9未开始/12进行中或待条件），只Q04全部子项排除，UID风险未修复。
旧31任务行/history/118catalogbody/maturity/oldevidence保留，无task/class4/goal完成，
第五至九类和全部其他原验收不豁免；timing/zero/overflow迁移须pre-1.0 review。

### 大类 4：L01/L02 paired discovery original IO retention（2026-10-07）

实现 `1ab05e3bcefd3926a402dc645f55c72d16aad6f4`，父 `6f9e4fd6187750a6a9a2f7416fd597990287adeb`；3 source files 134+/ 1-、4newtests。
[验收证据](validation/tunnel-paired-error-retention-2026-10-07.md)。仅stale paired branch
把成功旧reply映射Ok，已取得原Err直接保留；同poll clear/close不再吞sourceIO。
Clock/candidate/discovery/window/stopA-activeB rules、公有API/fields/errors/options/wire/
deps/features/lock/Core/bindings/旧asserts保持。ActuallocalhostTCP1byte后close，real
read_exact2byte取得UnexpectedEof后同poll completed managerclear或notifyclose；先前
publisheduserspace originalArc/joins/status/DuplexEOF/taskbaseline先恢复后regression。
4cases均connect1/USB1/paired1、no stalehandoff/ready；privateSource非真实mobdev2/DNS。

Wrong-old82506 terminal101/new2PASS2FAIL+old2PASS，production未动、453hash一致。
Development13051 terminal101原type/kind和native recovery已pass，但新断言错把直接EOF
归Disconnected；既有Core合同为Io（EOF可为hostfile截断），仅修新expected，Core/旧asserts
不改，exactsource/log已存。12590terminal0/all6matchingPASS。Final26492terminal0/
all7serial gates：CLI212/1ignored/minCLIalltargets/oldstack5/strictworkspace/fmt/full/
.venvtools73 PASS；**2736Rust/13Python，CLI1075/Core1496/public55，0FAIL/2既有ignored**。
MSRV1.80.1/66features/4groups/docs7+7/snippets3/C ABI/3.12 rebuiltwheel-sdist/help118/
raw38/host5/CLI95/backup3 PASS。22device NOT_RUN；Chromium/installed3.9/3.13 NOT_RERUN。
PatchSHA256 `0d92fcf30c3fc44e62f4975228dcad692d14147de054cf8946816fe8b3f7e56b`；actualiosSHA256 `84127d56cd68799bbe0a66aec08b72763c1afefe346af32dcb67266f75d37bc0`；commit=frozen453
hash，refsHEAD/status不变。Root独serialRust/独applysource、activevalidation无source/doc
edits；CARGO_INCREMENTAL=0，无cachedelete/push/publish/historyrewrite/device/credentials/
Apple/elevatedTUN。Ignored证据 `target/validation/paired-discovery-primary-2026-10-07/`。

有效30主项及全部children保持active：9完成/21未完成（9未开始/12进行中或待条件），
只Q04全部子项排除、UID未修复，其他原验收不豁免；旧31任务行/history/118catalogbody/
maturity/oldevidence保留。WholeScan postpaired clocks/cancel/failurebookkeeping、metadata/
HTTP/upgrades/blocking/startup/physicalfreshness/reuse/restart/manualepoch及其他L01-L04、
第五至九类继续，无主项/class4/goal完成，无maturity/设备/ACK/性能/平台认证。

Readonly follow-up review: this four-case profile covers completed clear/close
inside Source before its reply. A subsequent epoch change between the two paired
checks can still discard acquired Err at the second branch; that known race
remains open and will receive a separate regression/fix, not claimed accepted here.

### 大类 4：L01/L02 paired final epoch fence（2026-10-08）

实现 `aa3c3a46c19dd19175c879bcb44f5856d24259ec`，父 `257aecc00f8a78a6be023132abe9b0769043eaed`；3 source files 158+/1-、4newtests。[final paired fence evidence](validation/tunnel-paired-final-fence-2026-10-07.md)。
承接1ab05e3/257aecc记录的第二paired检查race：首次check通过后clear/close变更epoch，第二stale分支也直接保Err。Production只改该return，hook仅cfgtest，公有API/fields/enums/wire/clock/window/Core/bindings/features/deps及全部旧asserts保持。
ActualTCP partialEOF，首check通过后actualclear(firstpoll mustcomplete)/notifyShutdown；先published exactnativeArc originaljoins/status/DuplexEOF/taskbaseline/registry收尾后regression。Clear-success含unique nonemptytarget，close-success仅empty；不认证所有staleSuccess均Ok或零coordinator。
Wrong-old75395 terminal101/2PASS2FAIL且454hash不变；fixed60167terminal0/15PASS；Final52043terminal0/all7serial gates PASS，CLI216/1ignored/minCLIalltargets/stack5/strictworkspace/fmt/full/tools73。**2740Rust13Python，CLI1079/Core1496/public55，0FAIL2既有ignored**。
MSRV1.80.1/66features4groups/docs7+7/snippets3/C ABI/3.12 rebuiltwheel-sdist/help118/raw38/host5/CLI95/backup3 PASS；22device NOT_RUN，Chromium/installed3.9/3.13 NOT_RERUN。
PatchSHA256 `7bc7fb5db19d2a9266fad866b1e31fed6dd750a02087be1300bfaff07429cecf`；actualiosSHA256 `707fd3603787a1ef2fa2b594fdf3c3343dbf86127691fc3242056e3872ef431d`；冻结454products/refsHEADstatus不变。
Root独serialRust，activevalidation无source/docs编辑，CARGO_INCREMENTAL=0；无cachedelete/push/publish/historyrewrite/device/credentials/Apple/elevatedTUN；ignored证据 `target/validation/paired-final-fence-2026-10-07/`。
Bothstale branches已保Err；之后reconciliation/candidates仍在setupclock外等待，pairedprimary只归scanfuture、不跨outerdrop持久保留。Whole-scan total/cancellation/paired primary across caller drop,out-of-clock failure bookkeeping,metadata admission and all other original L01-L04/classes5-9 requirements remain.
有效30主项及全部children保持active：9完成/21未完成（9未开始/12进行中或待条件）；只Q04全部子项排除、UID未修复，其他原验收不豁免。无主项/class4/goal完成，无maturity/设备/ACK/性能/平台认证；旧31任务行/history/118catalogbody/oldevidence保持。

### 大类 4：L01/L02 manager snapshot admission（2026-10-08）

实现 `327b0ef38e43576f2313c51c597b5c16cb7abc01`，父 `061bf296653f8f1fcd8ab97bf4e29f33b6c44c65`；6 source files 743+/5-、14newtests；[snapshot admission evidence](validation/tunnel-snapshot-admission-2026-10-08.md)。
私有raw16,384rows/每族16MiB UTF-8、单UDID/host4096bytes、USB unique4096（包括未授权）、授权combined candidates4096及其自有ID+host16MiB。四Source消费点完整验证，USBbeforeattached-evict/selection/any；manualpaired beforefind。借用完整paired plan先于clone/ticket；raw/combined拒绝整批incomplete、立即droptargets、保未知mobdev2backoff、合法USB继续后返回ResourceLimit。SourceErr外层保持、staleSourceOk quiet；旧clock/ticket顺序/fourwindow/stopA-activeB/normal selection/defaultfallback/publicCoreSDKwiredepsfeaturesasserts保持。
47308 terminal101/19PASS5FAIL(7new2PASS5FAIL,17oldPASS)，455exacthash；初始unique测试已被旧coordinator cap拒绝、不能当旧fail，后续helper追加无newgate断言。24740/26PASS、93542/28PASS、72932/31PASS。最终12016 terminal0/all7SERIAL gates：CLI230/1ignored/minCLIalltargets/stack5/strictworkspace/fmt/full/tools73 PASS；**2754Rust13Python，CLI1093/Core1496/public55，0FAIL2既有ignored**。
Exact/over raw rows/unique/UTF-8 field/aggregate/combinedcount/combinedbytes、configuredunauthorizedUSB计数、actualHTTPpolicyreload只删indices、新freshpairedprefix无ticket/contact。两actualnative userspace fixtures：oversizedUSBtail时oldArc live/newArc retire、actualclear后invalidpairedSourceOk quiet；originaljoins/status/DuplexEOF/taskbaseline先恢复后regression。PrivateSource metadata不是mobdev2/DNS/device证据。
MSRV1.80.1/66features4groups/docs7+7/snippets3/C ABI/3.12 rebuiltwheel-sdist/help118/raw38/host5/CLI95/backup3 PASS；22device NOT_RUN，Chromium/installed3.9/3.13 NOT_RERUN。
PatchSHA256 `7faa7b3f9ecab3a12900dbf6cfc5567804427b73922b3d0267b434d29079a0a8`；actualiosSHA256 `24fcb89c22b621d619df6cf5b3f53d567ee2ff6924f54fc4bc6c666a9ad5f9f7`；冻结457products/refsHEADstatus不变。Root独serialRust，activevalidation无source/docs编辑，CARGO_INCREMENTAL=0，无cachedelete/push/publish/historyrewrite/device/credentials/Apple/elevatedTUN；ignored证据 `target/validation/tunnel-snapshot-admission-2026-10-08/`。
仅manager接受/复制的logical metadata边界，不认证Source/Core allocation/capacity/RSS/CPU/性能/长运行；initialpublicationclone/manualinput/globalgate-failure-cache-native-list-federationbytes及wholeScan total/cancel/pairedprimary跨outerdrop/failure-write继续。新增资源拒绝是pre-1.0 managed行为需review。
普通L01/L02 admission不是Q04的UID/fuzz/资源专项，UID未修复。有效30主项及全部children保持active：9完成/21未完成（9未开始/12进行中或待条件）；只Q04全部子项排除，其他原验收不豁免。L03-L04/第五至九类及所有原条件继续，无主项/class4/goal完成，无maturity/设备/ACK/性能/平台认证；旧31任务行/history/118catalogbody/oldevidence保持。

### 大类 4：L01/L02 deferred failure bookkeeping（2026-10-08）

实现 `4415be9a65921a346208f912dbe59e399e471591`，父 `14056003d583f174cb0bc577db577b41d3dcfd77`；7 source files 714+/9-、12newtests；[failure bookkeeping evidence](validation/tunnel-bookkeeping-2026-10-08.md)。
Failedcandidate先返回identity/permit，scanner沿用原UDID/Ticket持intent、四连接窗口结束后才排failed-outcome writer。Admission上限4096/ID+routehost16MiB派生logical边界；discovery15s/candidate15s与selection/individualcancel/oldtestAPIs和全部oldasserts保持。
非空flush共用一次checked absolute1s覆盖写锁等待和整批commit；empty无work，zero/overflow不写且slots恢复。Ticket内时间pre/post实际insert，late新row删除/oldrow原timestamp-count-identity-permit还原，两类guards释放后drop displaced；及时prefix可保留，非batch事务。StopA只跳A；不认证写锁等待即时cancel。
BookkeepingElapsed是scan metadata错误、不另记连接failure；newready须flush成功，原ready不因incomplete清除。原pairedSourceErr或Incomplete priority，original.context保directIOdowncast/CLIresource分类，真实Elapsed secondary独立可查且Debug脱敏。CallerDrop持久primary继续。
Backoff公式/count/identity/permit规则保持，last_attempt仍metadata commit时刻但移到窗口末，早期failure下次retry可能延后；newbounded rejection/timing为pre-1.0需review。Success内部failedremove仍可排writer，整体fairness/wholeScan不因此认证。
4actualnative fixtures first4realCore reservation/EOF失效时failure表read-held，第5actualnative仍进入、attachedpending=Unpublished/live、Drop后退役；heldflush先到期再释放reader，原paired实际EOF/CLIresource保primary+actualElapsed。所有originaljoins/status/DuplexEOF/taskbaseline/closedempty/permits先恢复后regression；无mobdev2/DNS/device证据。
Old13133 terminal101/0PASS4FAIL、458hash，actualnative recovery先完成。V1 29280=9PASS2FAIL/V2 82588=10PASS1FAIL仅newfixture观察同步/CLI分类/attachedphase错误，exactsrc/log归档、oldasserts/Core types不改。Disabledpostinsert69833 terminal101/0PASS2FAIL，证明outerElapsed不足；canonicalbytes恢复后加shared-two-writes行为例。
Final5784 terminal0/all7SERIAL gates PASS，CLI242/1ignored/minCLIalltargets/stack5/strictworkspace/fmt/full/tools73；**2766Rust13Python，CLI1105/Core1496/public55，0FAIL2既有ignored**。MSRV1.80.1/66features4groups/docs7+7/snippets3/C ABI/3.12 rebuiltwheel-sdist/help118/raw38/host5/CLI95/backup3 PASS；22device NOT_RUN，Chromium/installed3.9/3.13 NOT_RERUN。
PatchSHA256 `3cae15f13acb5092ee0e209a04c7091a1ed053e1b20b540deeeb975c348307e6`；actualiosSHA256 `2c72568a0835e2af5dbd976c534d409db8dc6727a013c35007c73753613e117e`；460products/refsHEADstatus不变；Root独serialRust、activevalidation无source/docs编辑、CARGO_INCREMENTAL=0，无cachedelete/push/publish/historyrewrite/device/credentials/Apple/elevatedTUN；ignored证据 `target/validation/tunnel-bookkeeping-2026-10-08/`。
Whole-scan total/global cancellation/durable paired or List primary across callerDrop,reconciliation/readiness/global metadata,successful-candidate writer fairness,L03-L04/classes5-9 and all other original acceptance remain. Source allocation/allocator/RSS/CPU/throughput/长运行未认证；ordinaryL01/L02不是Q04全部子项、UID未修复。
有效30原主项及全部children继续active：9完成/21未完成（9未开始/12进行中或待条件）；其他原验收不豁免。无主项/class4/goal完成，无maturity/设备/ACK/性能/平台认证；旧31行/history/118catalogbody/oldevidence保持。

### L01/L02：成功扫描结果的集中清理（2026-10-08）

实现 `6bceff76ce31bac1006b4e97f1d0d00a28731abc`，父 `b6af4fd`。
[验收证据](validation/tunnel-success-bookkeeping-2026-10-08.md)。成功连接不再在
窗口中排队写退避表；无竞争时立即归还原 slot，有竞争时释放连接许可后与失败
结果统一清理。原 row version 和 ticket 防止旧成功删掉晚到失败；同 poll 跨期限
恢复未观察的原 row，成功清理超时不再被误记为建连失败。

独立 worktree 并行实现/补回归、主 agent 集中集成和串行编译。冻结470产品文件，
隧道回归268通过/0失败/1既有忽略；完整统一入口一次通过，Rust2792/Python13，
0失败/2既有忽略。必需门禁、绑定制品及主机流程均通过；22真机行NOT_RUN。
其他L01/L02验收继续，本子项不完成主任务；Q04及全部子项保持排除。
下一批调度与具体条件见[当前工程调度](roadmap-current.md)。


### F01 / L01 / L02：显式设备 doctor 与手动准入（2026-10-08）

本地实现提交：手动 selector `f0ec223`，配对文件准入 `82c6c56`，设备 doctor
`71a59fc`；最终组合代码为 `71a59fc`。[完整主机证据](validation/device-doctor-manual-admission-2026-10-08.md)。
手动输入在 key/lock/Source 前按4096 UTF-8 bytes拒绝；原native owner保留和实际
收尾已覆盖。doctor在显式UDID/IOS_UDID下使用3个fresh只读existing-trust session，
字段省略与false分开，报告与最终machine failure类别一致，原native error/downcast
保留；无自动配对、tunnel激活、设置修改。DDI只观察signature，不认证mounted/usable。
Unix FIFO通过nonblocking open及opened-handle regular检查拒绝，普通文件symlink、
系统可读权限、1MiB上限和secret wiping保留。

CLI受影响回归1159 PASS/1既有ignored，imagemounter35 PASS，pair-record15 PASS；
最终统一full门禁PASS，Rust2832/Python13，0FAIL/2既有ignored。冻结476产品文件，
strict check/Clippy/MSRV、66features/4groups、docs、C ABI、wheel/sdist、73tooling、
118inventory、43raw captures、5host/95CLI/3backup全部通过。仅清理未使用incremental
cache，源码/产物/证据不变，所有build用CARGO_INCREMENTAL=0。22真机检查仍NOT_RUN。

RSD/route目前仍not_probed；既有RSD route的checked只读API在独立worktree开发，
不能据设计标完成。平台USB/remoted/目录、DDI真实可用性、物理拔插/服务重启继续。
OS metadata/regular file IO仍可卡住，spawn_blocking不能被物理取消，runtime退出
可能等待worker；FIFO修复不完成该项。L01/L02原bytes/生命周期/性能/平台验收保留。
仅交付主机子项，无主任务或整体目标完成；Q04与全部子项继续排除。下一批按
[当前调度](roadmap-current.md) 集成manager重连及AFC/Browse SDK前置。

### A03：有界文件读取与 Browse 原生前置（2026-10-08）

`655733c` 集中交付 AFC caller-limit 和 InstallationProxy checked Browse。
前者不依赖 file-size 声明、不返回截断成功，沿用单请求/独立 close 预算；
后者用一个 total 覆盖 request/all replies/parse，检查累计 wire/帧/条目并在
committed 取消/失败后拒绝全部旧入口复用。30 个新增真实协议/owned TCP 回归、
最小 afc/apps features 与完整门禁通过；准确源码/日志边界与迁移见
[证据](validation/bounded-service-read-2026-10-08.md)。完整运行含 pending manager
proposal，manager 的审查修正另行验收，未借此将其标为完成。

仅 A03 两个原生子项完成；Python/C common owner、bounded query、日志流、
context/async/GIL、取消/并发/关闭与 C ABI/示例仍待交付，主任务继续进行。
22 真机行 NOT_RUN，平台/物理/性能和全部原验收继续；Q04 全部子项排除。

### L03：manager 可配置自动重连 episode（2026-10-08）

`1a604af` 集中交付 serve 退避/attempt/total flags、有限持久 episode、
原 identity/epoch/version/permit/native-owner 边界与准确 INFO 状态。默认保留
legacy 30/60/120/240/300s；默认日志省略不能证明的累计数据，配置 episode 才
输出权威计数与期限。真实 A pending 到 1.5s total，同时 B 实际 HTTP GET/DELETE
各 <1s，原 joins/EOF/tasks/permits 恢复。26 新回归及完整門禁通过；
[证据与迁移](validation/manager-reconnect-2026-10-08.md) 记录原错误优先和
本地 expired budget 与未取得 native error 的区别。

仅自动 manager 主机子项完成；DELETE 取消旧 incarnation 后下一 scan 仍可能
重新发现，durable stop/manual directed-Wi-Fi intent 是下一工作项，Darwin watch
还需 fresh owner/RSD/services 与 baseline/nonce/count/gap 语义。既有 Tokio
serial/scan lock 保持，未声称每次重试前已完成所有 native joins。平台/物理/
性能/长期运行与 22 真机 NOT_RUN 保留，L03 和全部原主任务条件不降格。

## 2026-10-09：既有 RSD 观察与 Syslog framing 主机子项

`838c4ed` 完成 caller-selected IPv6/既有 userspace proxy 的 checked total RSD
观察和 doctor 接线，完整目录才支持 absence、metadata 缺失保留 unknown，目标标识
先核对再归属；不执行 advertised 服务或建立 tunnel。`f077674` 完成 Syslog 取消后
保留已消费前缀、EOF/原 IO/超长后 terminal 并在后续调用前拒绝复用。

集中完整门禁 Rust2923/Python host13、0失败/2既有忽略；最小 tunnel,syslog509、
48实际 CLI 原始输出场景通过。486受测产品文件冻结，最终提交 blob 与清单逐项一致。
[实现、迁移、失败修正与验收证据](validation/doctor-rsd-syslog-2026-10-09.md)。

F01/A03/L01 仅新增上述主机子项，主任务不标完成；22真机行仍 NOT_RUN，平台、
DDI usability、阻塞文件 IO、绑定和设备工作流原验收继续。下一批 SDK 共同 owner/
checked existing-trust、L03 manual route/跨 scan stop 已划定独立 worktree 边界，
主 agent 集成、串行编译并冻结被测源码。Q04及全部子项继续排除。

## 2026-10-09：SDK 共同 owner 与 checked existing-trust 前置

`2b906d7` 完成有限 owned service observation、单一绝对期限、取消、无等待队列的
owner/shared 准入、close/last-handle Drop 和实际 lease 回收。checked lockdown 共用
QueryType/StartSession/真实 TLS/StartService 时钟，严格 Error-first 和必要元数据；
原 native IO/Elapsed/rustls source 保留。调用方已有 stream 和 Arc<PairRecord>；
未建立设备 factory/query 或 Python/C 常用 SDK，StartService 的远端效果没有回滚证明。

初始完整入口在已通过 fmt/check 后发现测试多余解引用；修正后串行恢复所有剩余
必需门禁，退出0，Rust2953/Python13，493产品路径及提交blob与冻结manifest一致。
完整入口本身的失败和分段验收分别保留，未伪称单次 full 成功。
[边界、失败修正与验收证据](validation/sdk-owned-session-2026-10-09.md)。

A03/L01 仅此原生子项验收，不标主任务完成；22真机项仍 NOT_RUN，原全部绑定、
工作流、平台、OS worker与协议验收继续。Q04及全部子项排除，有效30项9完成/21未完成。

## 2026-10-09：F03 实际 pinned-client bridge 框架

`3e697fd` 在生产 authenticated/protected Rust HTTP/WS bridge 上运行固定
Puppeteer25.12.0 和 Playwright1.63.0。Puppeteer完成目标发现、附着、真实合成上下文
选择/求值、未知context拒绝、分离/断开；Playwright实际下载初始化得到原-32601。
首次真实客户端发现缺少browserContextId导致更早断言，已补既有单一默认逻辑context
标识；未提供上下文创建、隔离或虚假下载成功。实际child取消、wait、原pipes/native
peer EOF/joins与bounded trace分别验收。standalone假CDP探针仍仅请求发现工具。

初始full fmt/check通过后发现fixture冗余lifetime；修正后剩余必需门禁分段退出0，
Rust2954/Python13通过。ordinary5ignored中三个Node gates在final499路径另外执行
3PASS/0FAIL/0ignored；所有提交blob与冻结manifest相同。完整入口原失败和分段证据
均保留，不伪称单次full成功。[协议证据、下载真实实现方案与复现](cdp-client-workflow.md)。

F03主任务仍进行中，F04各域、真实页面/JSContext/设备与平台原验收不缩减；22真机
NOT_RUN。Q04及全部子项继续排除；主任务数量不因新增测试/导出增加而改变。

### 2026-10-09 F03 原验收收口复核

按原 F03 定义逐项核对 `3e697fd` 的既有实际证据，主任务完成：固定
Playwright/Puppeteer 库、可重复 fixture、生产 protected Rust HTTP/WS bridge
到 owned synthetic WebInspector、实际请求/event/关闭 trace 和原 child/pipe/peer
回收均已交付。Puppeteer 覆盖初始化、发现、附着、上下文、求值和分离；Playwright
产生可复现、关联的 Browser.setDownloadBehavior/-32601。官方协议依据与实际下载
实现方案见 [客户端说明](cdp-client-workflow.md)。

原 F03 明确接受缺少初始化语义时提供协议证据与真实实现方案，因此此前以完整
Playwright 成功或 F04 真机条件延后 F03 收口扩大了原要求，本次更正当前状态。
Playwright 初始化 observation 仍 FAIL，负合同 gate PASS 不能转换成工作流成功。
F04 的 console/context destruction、Page/iframe、Input/截图、Fetch/下载、指标
和代表真机验收仍分别未完成，capability maturity 不升级。有效任务现为10完成/
20未完成；这是按原工作项交付收口，不由测试数量推导。

用户已明确真机测试后续统一安排；22真机行继续 NOT_RUN，其他平台缺口和 Q04
全部子项排除保持。主机可执行工作、验收和本地提交继续；不推送、发布或部署。

### 2026-10-09 有限 Rust device query 子项验收

`6f582c9` 完成显式目标、already-loaded PairRecord、fresh usbmux/Lockdown/TLS
GetValue，有限 selected/root projection、同一总期限/取消/并发和累计回复/metadata/
owned输出预算。21新query fixtures与2pair-byte用例；失败根因和最终门禁见
[验收证据](validation/sdk-device-query-2026-10-09.md)。

最终剩余必需门禁 exit0/863.130s：Rust2977/Python13、零失败；workspace check保留
先前31.1s单独证据，test-only TLS素材模块共享修正后的完整Rust/strict lint/其他阶段
均实际执行，505冻结path与实现提交blobs一致。335 minimal与21专项此前通过；五个
ordinary ignored不代表客户端执行。22真机行NOT_RUN，用户后续统一安排；A03绑定/
应用/文件/日志及原例子/平台/设备验收继续。无主任务或maturity升级，Q04所有子项排除。

### L04：manager/native 资源基线主机子项（2026-10-09）

[验收与归因](validation/manager-resource-baseline-2026-10-09.md)，
[结构化结果](validation/manager-resource-baseline-2026-10-09.json)。
修改前 `2964c1c`、修改后 `e53e0bb` 使用逐字节相同的fixture、同工具链/
all-features/debug输入，1/4/16和30s/104轮测量已比较，原owner收尾通过。
另用原二进制进行60s/208及207轮分配器/匿名与文件页诊断，末10秒RSS稳定，
原FD/socket/任务/registry/permit恢复；记录测得延迟和从基线派生的主机比较阈值。
此子项完成额度中断时的测量与资源归因；L04主任务的吞吐、文件、服务流、
平台和真机验收继续，能力成熟度不升级。

## 2026-10-12：Q02 原范围开发收口

通用Windows capture/agent已提交300ad41；原FUP核查补全agent最终gate、普通60秒
上限、capture异常清理状态和文件size/hash事实。受影响65方法61通过/4平台跳过；
主机runner5 PASS/22 NOT_RUN。所有生产observer/adapter均已有接线，按用户开发
口径Q02计完成；真实运行条件不升级PASS，旧来源限制保留。账本23交付/6缺开发/
L02待完整收口，有效30，Q04全部排除。完整边界见[核查](validation/q02-development-closeout-2026-10-12.md)。

## 2026-10-12：L02 原error payload采样归因子项

跨retry receipt/abandoned scan/native registry的原错误联合采样已接线，已知String
capacity按原buffer去重；未知payload/未join native owner/active scan/截断都显式记录。
不格式化任意原错误，不改变原对象/source/receipt，不据槽位数宣称字节上限。
新8项、受影响tunnel359项与实际readonly-policy CLI门禁结果和剩余全局字节准入
实现见[证据](validation/manager-error-payload-2026-10-12.md)。L02保持未关闭，账本23/
6/L02待收口，有效30；设备和其它平台运行条件继续，Q04全部排除。

## 2026-10-12：L02 Source snapshot capacity准入子项

短String的大reserve和空Vec容量不能再绕过快照准入：原16MiB文本额度按capacity
计费，checked合并Vec存储并限32MiB。原字段/candidate长度合同保持，四个生产
消费点共同应用。受影响snapshot34项通过（新6项），原native owner/epoch/错误
与严格CLI Clippy已验；来源分配峰值及统一全局字节准入仍缺，L02不关闭。
详情与实际CLI/源码提交证据见[容量交付](validation/tunnel-snapshot-capacity-2026-10-12.md)。
账本23交付/6缺开发/L02待收口，有效30，Q04及全部子项排除。

## 2026-10-12：L02 raw snapshot/publication共享额度子项

四个Source消费点的raw snapshot与published metadata接入128MiB共享已知charge池，
原64MiBpublication子限保留。snapshot caller Drop与拒绝释放原charge；publication
global/sub-limit两份permit由table和original native owner共享，直到两者最后释放。
原metadata/native/epoch/deadline/source门禁集中369通过/2原ignored（含新4项），
详情与其余error/history/candidate/global缺口见[证据](validation/tunnel-global-known-metadata-2026-10-12.md)。
L02未关闭，账本23交付/6缺开发/L02待收口，有效30，Q04全部排除；无设备操作或推送。

## 2026-10-12：L02 coordinator key/gate/backing同池归属子项

原canonical gate的key容量、inline gate和实际Vec backing接入同一128MiB bank；
新旧扩容backing同时准入，拒绝不改变原gate/ticket/table。重复ticket共享原charge，
clear/stop不提前退款，最后gate owner释放才归还。原4096 namespace/pruning、串行
与epoch规则保留。新增4项及受影响tunnel373项通过，2原ignored保留。
生产CLI/其它门禁与精确边界见[证据](validation/tunnel-coordinator-bytes-2026-10-12.md)。
原error/history/candidate/manual route与其它backing仍缺完整准入，L02保持未关闭；
账本23交付/6缺开发/L02待收口，有效30，Q04及全部子项排除。

## 2026-10-12：L02 USB selection backing准入子项

生产USB selection改为借用索引Vec，移除临时HashMap/key clone；实际索引/输出
Vec capacity与原raw reservation同池准入，首USB/最后网络/首次identity调度顺序
和原String buffer保持。新增5项及受影响tunnel378项通过，2原ignored保留；
其它门禁与准确剩余见[证据](validation/tunnel-selection-bytes-2026-10-12.md)。
L02及账本23/6/待完整收口不变，有效30，Q04及全部子项排除。

## 2026-10-12：L02 paired plan backing准入子项

paired候选完整规划的借用HashSet改为同池准入的实际capacity索引Vec；首host、
原调度顺序、USB/权限过滤和原count/text限制保持。索引拒绝为Incomplete，
释放拒绝批次，不影响合法USB候选或认证未知paired detach。新增5项及受影响
tunnel383项通过，2原ignored保留；生产fixture证实一字节不足仍发布USB且不建
paired ticket/连接。其余门禁与剩余边界见[证据](validation/tunnel-paired-plan-bytes-2026-10-12.md)。
L02及账本23/6/待完整收口不变，有效30，Q04及全部子项排除。

## 2026-10-12：L02 candidate reconciliation索引准入子项

failure/retry reconciliation改为借用原candidate Route/native ID的已准入Vec索引，
不再clone host或保留临时HashMap。重复key最后记录语义、原identity规则保持。
新增5项和受影响tunnel388项通过，2原ignored保留；生产索引拒绝保持原backoff
version/slot/identity并使Source建连零调用。其余门禁与完整剩余见[证据](validation/tunnel-candidate-index-bytes-2026-10-12.md)。
L02仍未完整收口，账本23/6/待收口、有效30与Q04全部排除不变。

本项复核发现先前通用HTTP refresh unit默认NativeSource可进入主机设备发现；
373/378/383与首次388的PASS不能用于认证全部Rust测试隔离。默认unit Source已
改为NoDevices，生产NativeSource与专用localhost native原joins/strict EOF保持；
修正后388通过/2原ignored。原日志保留，先前具体设备状态NOT_OBSERVED，见
[隔离更正](validation/tunnel-fixture-isolation-2026-10-12.md)。

## 2026-10-13：L02 candidate输入/backing共享owner子项

原candidate输入按String capacity及inline Input同池准入，USB原buffer charge拆分
转移，不双重计费；Source route和延期intent key副本预留。candidate Vec增长/iterator
backing准确归属；paired输入整批准入后才创建新tickets/合入USB window。table、原
native registry、Source原错误context及延期intent共用Input直到最后owner释放。
原Io/downcast、原native joins/严格EOF、取消/epoch/权限重载断言保持；首次396/1失败
修复每行权限重查后397通过/2原ignored，新增9项均通过。严格Clippy与其它出口见
[证据](validation/tunnel-candidate-owners-2026-10-12.md)。原error/history创建端、outcome
collectors/manual副本/其它backing及完整刷新规则仍缺；账本23/6/L02待收口、有效30
及Q04全部排除不变，未改为主项完成。

## 2026-10-13：L02 outcome/receipt/displaced批次准入与原错误收尾

Source window前按候选数准入三份实际Vec backing，manual批次/完成版本列表亦准入。
原scan Session持共享receipt owner穿过内部marker→实际timer/epoch结果、manual收尾、
writer、发现错误合并和readiness；原Source对象与phase完成状态保留，返回报告持有
已准入Vec至caller最后释放。新增7项/受影响tunnel404通过、2原ignored，原joins/严格
EOF/对象/权限等断言不放宽。完整门禁、初始fixture失败与精确剩余见[证据](validation/tunnel-write-batches-2026-10-13.md)。
原error payload创建端/其它history backing、manual副本/其它集合与完整刷新规则继续；
L02仍待完整收口，账本23/6/待收口、有效30与Q04全部排除不变。

## 2026-10-13：L02 manual desired route共享准入owner

Manual desired的原RouteIntent buffer由已准入RetainedRoute持有，Snapshot/rollback
clone同一Arc而不复制或重复收费。按原capacity/inline size同池准入，替换先检clock/
grant再准备owner，额度拒绝保持原version/watch/pending；迟到回滚恢复原Arc，stop/
clear不提前释放caller旧Snapshot，最后owner归还。新增4项与受影响tunnel408通过、
2原ignored，严格Clippy与其余出口见[证据](validation/tunnel-manual-route-bytes-2026-10-13.md)。
原error payload创建端、listing key/backing与其它副本/集合/native/history/internal
backing及完整刷新规则继续；账本23/6/L02待收口、有效30与Q04全部排除不变。


## 2026-10-13 L02 desired listing子项

原key capacity与实际Vec backing进入同一已知元数据bank，scan借用key，manual
success writer转移原key和permit；5新增及tunnel413通过、2原ignored。严格Clippy、
CLI build、只读manager/host smoke通过，真实设备NOT_RUN；原主项未关闭。
见[验证](validation/tunnel-desired-list-bytes-2026-10-13.md)及
[原错误创建端复核](validation/tunnel-error-creation-contract-2026-10-13.md)。


## 2026-10-13 L01/F01 profile接线与验证

SpringBoard原stream进展/取消/失败所有权、共享总期限及逐IO fence已接线；CLI
request-timeout-ms正值默认30秒，minimal/allfeature均21通过，parser1通过，
严格Core/CLI Clippy/build通过；建连/其它L01 profile仍缺，见
[验证](validation/springboard-lifecycle-2026-10-13.md)。

Doctor显式目标追加已有协议personalization manifest观察，host配对目录只枚举
metadata，平台USB malformed/缺失不冒称Ok；60项fixture通过、1原ignored，
严格CLI Clippy/build和119项capability inventory通过。未运行真实设备/平台发现；
完整本地BuildManifest/组件/摘要/签名范围仍缺，原F01不关闭，见
[验证](validation/doctor-manifest-directories-2026-10-13.md)。


## 2026-10-13 L02 scan自有胶囊预算子项

Source首次poll前原子预留原context/cleanup/abandoned payload，permit随实际
胶囊及最后caller/history owner保留；原error对象/chain、native joins及EOF保持。
9新增及整组422通过、2原ignored；严格Core/CLI Clippy/build/只读CLI和host通过。
原不透明payload及其它heap/完整刷新仍缺合同，不关闭L02，见
[验证](validation/tunnel-scan-capsule-bytes-2026-10-13.md)。


## 2026-10-13 F02终态CI与最终receipt子项

Core共同终态验证不再把未完成/未知/异常结果写成成功；保留case及unexpected
失败计数，重跑不能覆盖初次失败。最终stdout在原cleanup/report/sidecar之后，
stdout失败的唯一sidecar重写共用原artifact clock。Core5新增/19原integration、
CLI15/10/11及实际CLI26方法通过，严格Clippy/build/fmt通过；原错误/EOF/任务
join保持。现代全流程、结果准入和整体Rust组合等原范围仍缺，主项不关闭，见
[验证](validation/f02-terminal-integrity-2026-10-13.md)。


## 2026-10-13 L02 failure writer自有payload子项

实体owner/context payload在Source connection window之前预留，未用context完成
即退款，实体份额保留到最后owner；相关9和整组424通过/2原ignored，严格CLI
Clippy/build、readonly/host/fmt通过。完整原error/native/history/backing及刷新
规则继续，不关闭原L02，见[验证](validation/tunnel-write-payloads-2026-10-13.md)。


## 2026-10-13 L02 native registry结构预算子项

manager同池实际Vec capacity与自有Registry sizeof在Source前收费，前台与
standalone沿用显式bank；生产构造失败返回，最后原Registry/Reservation owner
退款。4新增/整组428通过2原ignored，六组前台57与实际CLI31通过，严格CLI
Clippy/build/fmt及readonly/host通过。完整opaque/error/native/history及freshness
仍缺合同，不关闭L02，见[验证](validation/tunnel-native-registry-bytes-2026-10-13.md)。


## 2026-10-13 L02 retry/history自有存储准入

同池准入Ledger body、真实Vec backing、Box删除节点、receipt/Original/调用方
wrapper与实际copied key/route capacity；Source前完成，取得原错误后的历史落地
和remove/rollback不再申请新额度。first/latest共享原对象，原source/downcast
保持，最后实际node/caller owner释放才退款。诊断根快照收费，拒绝显式truncated。
4项新增及受影响tunnel433通过、2既有ignored；完整门禁及来源见
[验证](validation/tunnel-retry-owned-storage-2026-10-13.md)。任意dyn Error、
Arc/anyhow私有分配、native/runtime/backing及完整刷新仍缺；不关闭L02。
账本23交付/6缺开发/L02待收口、有效30和Q04全部排除保持。


## 2026-10-13 F04 native context关闭与发现刷新接线

原生已解析Listing/应用disconnect提供关闭证据；numeric page身份稳定，
独立刷新保留exact UUID route到原child join、queued reply/context与最终销毁
交付后再detach。channel/deferred共用64许可，join/flush/detach同一2秒绝对预算。
完整无关事件边界取消恢复原事件，partial native IO继续失效；原EOF/cleanup
源和task/native join断言保持。Core110、CLI184及pinned真实client10通过，
严格Clippy/build/fmt和readonly/host通过，见
[验证](validation/cdp-native-context-closure-2026-10-13.md)。public nonflat与
Playwright noDefaults仅本Runtime子项；默认download初始化拒绝保持。
Runtime console及Page/iframe/Input/Fetch/指标完整范围继续，不关闭F04；
账本23/6/L02待收口、有效30、Q04全部排除保持。


## 2026-10-13 L02 扫描起始publication快照收费

initial publication索引以真实KnownVec backing和copied String capacity
接入共享额度，USB/paired Source前完成；lookup借用原Arc，最后快照owner
退款，任意准入失败完整释放临时存储。原ListError/USB detach/paired完整
视图只作用于exact initial Arc的规则保持。3项新增及tunnel436通过、
2原ignored，见[验证](validation/tunnel-scan-publications-bytes-2026-10-13.md)。
其余manual/scan索引和key副本、error/native/runtime私有分配及完整刷新
继续；L02与23/6/待收口账本、有效30及Q04全部排除保持。


## 2026-10-13 L02 manual/suppressed扫描索引字节归属

用实际收费KnownVec row references/排序位置及suppressed key references
替换未收费HashMap/HashSet backing；borrow保留原desired key/ticket/route
身份，排序查找与捕获顺序迭代分开，部分准入失败不发布row，实际空backing
到最后index owner释放退款。原tunnel436及新增2项通过，严格CLI
Clippy/build/fmt、readonly1及host5/22NOT_RUN通过，见
[验证](validation/tunnel-scan-desired-index-bytes-2026-10-13.md)。其余索引/key
副本、原error/native/runtime私有allocation与完整刷新仍缺，不关闭L02；
账本23/6/待收口、有效30及Q04全部排除保持。


## 2026-10-13 L02 USB唯一身份校验临时backing

未收费HashSet改为实际收费sorted Vec<usize>；先验证原字段/容量、取得raw
费用，再取得临时index实际费用并全raw计数。原4096unique及16384raw上限、
UID buffer与发现顺序保持，成功只返回raw owner，失败释放两者。新增2项
及tunnel440通过、2原ignored，严格CLI Clippy/build/fmt、readonly1和
host5/22NOT_RUN通过，见[验证](validation/tunnel-usb-identity-index-bytes-2026-10-13.md)。
Source返回前分配、其余索引/key/error/native/runtime及完整刷新仍缺，
不关闭L02；23/6/待收口、有效30和Q04全部排除保持。


## 2026-10-13 L02 raw/selected USB attachment索引

raw attached集合和selected device-ID映射统一为borrowed UsbDevice slice+
真实收费sorted Vec<usize>，binary UID范围中匹配exact numeric ID，last
输入lookup保持。不同publication UID范围不重叠，不在table锁内进行
rawRows×pubRows全扫描或排序/分配。两个owner按原raw/selection阶段释放，
新增2项及tunnel442通过、2原ignored，旧quota/Source/error/native/EOF
断言未改，严格CLI Clippy/build/fmt和readonly1/host5/22NOT_RUN通过，见
[验证](validation/tunnel-usb-attachment-index-bytes-2026-10-13.md)。其余集合/
key/原error/native/runtime及完整刷新仍缺；L02/23/6/待收口、有效30和
Q04全部排除保持。


## 2026-10-13 L02 manual pending/planned暂停改动收口

pending_mobdev2/planned改为真实收费KnownVec backing，借用原desired
UID/ticket/host pin，不增加这些副本。pending在browse前准入，planned
拒绝按Incomplete处理，释放部分plan/raw费用，保留USB window和原
manual/USB publication。原tunnel442及新增2项通过，targeted3包含1原
方法；严格CLI Clippy/build/fmt、readonly1及host5/22NOT_RUN通过，见
[验证](validation/tunnel-manual-paired-plan-bytes-2026-10-13.md)。原error/private
allocation、seen/observed/失败表backing及完整刷新仍缺，不关闭L02；
账本23/6/L02待收口、有效30、Q04全部排除保持。


## 2026-10-13 L02 seen/paired observed 已知存储

seen以真实收费KnownVec共享原Input/UID，coordinator拒绝后的身份保留到扫描
结束；paired完整view借用原UID/host的排序位置，backing在Complete/ticket前
整批准入，拒绝保持Incomplete及USB window。新增6项和整组450通过、2原ignored；
一处等价自动解引用lint修正后预算14项/严格CLI Clippy/build/fmt/readonly1/host5
通过，22行NOT_RUN。原backoff/UID/source/native joins/严格EOF断言保持，见
[验证](validation/tunnel-seen-observed-bytes-2026-10-13.md)。failure额外owner与
原error/private/native存储及完整freshness继续；L02未关闭，账本23/6/待收口、
有效30和Q04全部排除保持。


## 2026-10-13 L02 failure 表原 owner 准入与回滚

Source前准备收费Box<Node>/共享route owner，移动Input已预留的唯一outcome UID，
实际destination backing先保留USB window；paired新增node整批在ticket前准入。
writer/成功删除/失败pruning移动和还原同一node，无key副本或后quota，最后owner
退款。原fence/错误/EOF/joins不放宽，462通过、2原ignored；最终解包消费lint修正
后成功清理11/严格CLI/build/readonly1/host5通过、22NOT_RUN，见
[验证](validation/tunnel-failure-table-owners-2026-10-13.md)。nonempty manual cleanup
晚准入、publication/private error/native存储与完整freshness等继续；L02未关闭，
账本23/6/待收口、有效30及Q04全部排除保持。


## 2026-10-14 L02 publication pruning 原 entry

真实临时KnownVec backing同池准入，predicate/watch分开，late/stale/unwind
恢复原key/value/native/费用，成功原owner在table guard外析构；四条原生产
pruning条件保持。7新增、tunnel469/预算16、严格CLI/build/readonly1/host5
通过，2原ignored/22NOT_RUN，见[验证](validation/tunnel-publication-retain-owners-2026-10-14.md)。
publication原backing、manual cleanup晚准入、private/native创建合同与完整
freshness仍缺；L02未关闭，账本23/6/待收口、有效30、Q04全部排除不变。

## 2026-10-14 L02 manual success Source 前准入

第一 Source poll 前准入4096项真实success-only/Completed backing和Preflight payload，
canonical原key随最后gate/ticket持费。晚收尾读取当前exact epoch/desired/pending
proof，authority→ticket writer/clear fences与晚clock原owner回滚已接线；Source
期间new UID/latest version零额度可收尾，idleAutomatic pruning保留。新增12项，
最终tunnel481/strictCLI/read-only manager1/host5通过，2原ignored/22NOT_RUN。
首次三项quota阶段失败、strict字段命名失败和续轮不完整日志均保留，见
[验证](validation/tunnel-manual-success-preflight-2026-10-14.md)。原error/private/
native/runtime和完整freshness等继续；本子项不关闭L02，账本23/6/待收口、
有效30、Q04全部排除保持。只本地提交，无设备操作或推送。
## 2026-10-14 L02 retry selector 创建前归属

生产retry借用已准入shared Identity，去除无owner的临时route副本；receipt/episode
真实key/route与payload先同池准入再复制/构造。原Source/retry/owner/error规则
保持，新增3项actual容量/原buffer/一字节拒绝/最后report退款覆盖；tunnel484、
strictCLI/readonly manager1/host5通过，2原ignored与22NOT_RUN保留，见
[验证](validation/tunnel-retry-selector-owners-2026-10-14.md)。publication/private
allocation/native/runtime与完整freshness仍缺；不关闭L02，23/6/待收口、有效30、
Q04全部排除不变。只本地提交，无设备操作或推送。
## 2026-10-14 L02 native RX buffer 归属

原Reservation向activation传递同一manager bank，真实header/packet Vec capacity
在创建前准入，clone-free buffer随queue/device/RxToken/closure最后owner退款；
原typed error/Ledger/strictEOF/taskjoin保持。新增10项，core tunnel101、activation4、
classification12、CLI485及strict core/CLI/build/readonly manager1/host5全部通过，
2原ignored/22NOT_RUN保留，见[验证](validation/tunnel-native-rx-storage-2026-10-14.md)。
Tx/socket/chunk/backing/private allocation/publication及完整freshness继续；L02
未关闭，128MiB非完整heap/RSS，23/6/待收口、有效30和Q04全部排除保持。
## 2026-10-14 L02 当前 RSD 目录与原 lease 更新

生产alive path读取当前checked directory，SDK resolver使用new ports；原initial
API/cause保持。非lease exact proof和原绝对min clocks、Source前cause capsule，
cache/table/registry metadata原位swap/rollback与最后fees owners已接线，17新tests。
Core RSD4/18/34与CLI498通过，等价lint整理后affected16、strictcoreCLI/build/
Source0 manager1/host5通过，2原ignored/22NOT_RUN保留，见
[验证](validation/tunnel-rsd-current-owners-2026-10-14.md)。raw/private parser/native/
publication存储与完整physical刷新/reuse/restart规则继续；同UID/ports不证明远程
incarnation，无虚构ACK。L02不关闭，128MiB非完整heap/RSS，23/6/待收口、
有效30与Q04全部排除保持。只本地提交，无设备操作或推送。
## 2026-10-14 L02 publication 真实表/node存储

唯一publication表为actual slot Vec/Box<Node>，global/sub双池先准入再分配，
原key/node/value/incarnation/slot rollback与paid intrusive removals接线；所有
list/query/clear/server Owner在guard外释放原owner，native/freshness费用仍随原
最后owner/joins。7table/3Epoch与真实native Source0/zeroquota门禁覆盖，完整CLI504
通过；最后cfg-only修复后affected46/strict/build/readonly manager1/host5通过，
2原ignored/22NOT_RUN保持，见[验证](validation/tunnel-publication-table-storage-2026-10-14.md)。
未收费的其它copies/manager/private/native/rawparser存储与物理刷新规则继续；
128MiB非完整heap/RSS、L02主项不关闭、23/6/待收口、有效30/Q04全部排除不变。
## 2026-10-14 L02 native Tx/socket/chunk存储

metered Tx在提供token前准入整MTU与真实queue，拒绝持原fatal再stop/no token；
原closure返回值/partialcursor/buffercapacity最后owner保持。真实queue peak、
connection backing、两个1MiB socket与4096/65536 chunks接线，socket先drop再
退fee。9新tests，core110/activation4/CLI504与strictcoreCLI/build/Source0 manager1/
host5通过，2原ignored/22NOT_RUN保留，见
[验证](validation/tunnel-native-transmit-storage-2026-10-14.md)。SocketSet内部backing/
Tokio/task/report/private/rawparser及其它原scope继续；128MiB非完整heap/RSS、
L02不关闭、23/6/待收口、有效30与Q04全部排除不变。只本地提交，无设备操作。
## 2026-10-14 L02 SocketStorage actual backing

真实dynamic SocketStorage Vec先准入增长peak，原whole Meta/socket/handles移位保持，
borrowed scope views不跨await/growth；zeroquota空位复用与lastowner费用接线。4新
门禁及core114、CLI504/strictcoreCLI/build/Source0 manager1/host5通过，2原ignored/
22NOT_RUN保留，见[验证](validation/tunnel-socket-storage-2026-10-14.md)。runtime/
Tokio/private/task/report/rawparser和其它originalscope继续；128MiB非完整heap/RSS、
L02未关闭，23/6/待收口、有效30与Q04全部排除不变。只本地提交，无设备操作。
## 2026-10-14 L02 channel actual message backing

manager runtime/client channels使用实际fixed message backing/body，创建前准入，
原FIFO/blocked send取消/close/最后value与body owner保持；Nonebank保留Tokio。
7新tests/core121/最终7复验/CLI504和strictcoreCLI/build/Source0 manager1/host5
通过，2原ignored/22NOT_RUN保留。旧Tx runtime额度补齐SocketStorage，并用
实际Tx hook证明MTU-1拒绝阶段，原fatal/joins/EOF不弱化，见
[验证](validation/tunnel-owned-channel-2026-10-14.md)。waiter/Arc/task/report/error/
rawparser等其它scope继续，128MiB非完整heap/RSS，L02未关闭，23/6/待收口、
有效30与Q04全部排除不变。只本地提交，无设备操作或推送。
## 2026-10-15 L02 native owner body/backing

metered userspace Ledger/SharedChildTaskSet应用body与真实四task-handle Vec在
worker前准入，费用随最后owner/backing；cleanup snapshot/Debug不再创建临时
Vec，原causes/identity/immutable report/pendingjoins/EOF保持。新增5项通过，
修正旧Tx fixture保留真实owner费用直至两个handle销毁；最终core126/CLI504及
strictcoreCLI/build/Source0 manager1/host5通过，2原ignored/22NOT_RUN保持，见
[验证](validation/tunnel-native-owner-bodies-2026-10-15.md)。Tokio/control/future/
JoinSet/Arc/report/private/rawparser等其它scope继续，128MiB非完整heap/RSS，
L02不关闭、23/6/待收口、有效30与Q04全部排除不变。只本地提交，无设备操作。
## 2026-10-15 L02 native future payloads

四parent与实际local proxy child在原spawn/JoinSet前按真实Box<F> payload准入，
原pinned future析构后退款，四parent全准入后才有worker；child不足保留原fatal/
joins/EOF。新增5项及core131/CLI504、strictcoreCLI/build/Source0 manager1/host5
通过，2原ignored/22NOT_RUN保持，见
[验证](validation/tunnel-native-future-payloads-2026-10-15.md)。Tokio TaskCell/
控制/结果、JoinSet/Arc/waiter/report/private/rawparser等其它scope继续；128MiB
非完整heap/RSS，L02不关闭、23/6/待收口、有效30与Q04全部排除不变。仅本地提交。

## 2026-10-15 L02 raw H2/XPC buffers and borrowed arena

原bank沿checked prepared生产路径准入真实received payload/demux replacement/
XPC body/borrowed Node+Edge数组，原slice/clone最后owner与两遍解析直接创建
catalogue接线；原四bootstrap/absoluteclock/nativeerrors/initialobservation保持。
新增17项，XPC141/prepared4/existing18/core131/CLI504及strictcore通过；最终
构建、受测hash/commit和smoke见
[验证](validation/tunnel-raw-parser-storage-2026-10-15.md)。Bytes私有ownerheader、
H2collection/outbound/future/initialRSD/Tokio/Arc/waiter/report等其它scope继续。
128MiB非完整heap/RSS；L02不关闭、23/6/待收口、有效30与Q04全部排除不变。
仅本地提交，无设备操作或推送。

## 2026-10-15 L02 raw operation boxed futures

H2七个operation与RSD五个原Box在分配前按实际future payload准入，原pin/
借用/取消/析构、deadline/guard和typed native result保持。新增6项与
XPC147/prepared4/existing18/core131及strictcore通过；最终CLI/feature/build/
受测hash和Source0 smoke见
[验证](validation/tunnel-raw-operation-futures-2026-10-15.md)。raw payload配额
fixture明确在真实payload creation点触发len-1，避免新future费改变目标阶段。
H2collection/outbound/initialRSD/privateBytes/Tokio/Arc/error/report等继续；
128MiB非完整heap/RSS，L02不关闭、23/6/待收口、有效30与Q04全部排除不变。

## 2026-10-15 L02 H2 quota refusal after progress

真实payload/demux allocation拒绝两项先在原guard复现失败，再修复progressed且
原ResourceLimit的invalidation；原error/source指针、严格EOF/fullrefund以及
尚未IO的原连接可用性保持。XPC149/prepared4/existing18/core131和strictcore
通过；新源码CLI相关freshness/strictCLI/build/Source0 smoke与前项完整504
证据的准确边界见[验证](validation/tunnel-quota-progress-2026-10-15.md)。
其余L02完整归属/预算/freshness继续；128MiB非完整heap/RSS，23/6/待收口、
有效30与Q04全部排除不变，仅本地提交。

## 2026-10-15 L02 pending DATA actual ring backing

原bank下pending DATA使用实际Vec<Option<DataFrame>>环形backing，创建前准入
与old/new同时持费；原payload/flags/FIFO、零配额frontrollback、clear持费及
最后slice owner保持。原1024frame/stream/byte limits和Nonebank deque不变。
新增9项、XPC158/prepared4/existing18/core131与strictcore通过；最终CLI/构建/
受测binary/hash/Source0 smoke证据见
[验证](validation/tunnel-pending-data-storage-2026-10-15.md)。其它H2maps/sets/
queues/outbound/private运行时与原freshness继续，128MiB非完整heap/RSS；
L02不关闭、23/6/待收口、有效30与Q04全部排除保持，仅本地提交。

## 2026-10-15 L02 flow-control actual counter storage

pending receive credit/outbound stream window使用真实banked key/value backing，
批次key准入先于counter reset/HEADERS，old/new peak/原ResourceLimit保持；
SETTINGS验证后独占原位更新移除scratch Vec，两帧WINDOW_UPDATE使用inline
13-byte字段。原wire/31bit/signedwindow/限制/EOF和Nonebank HashMap语义保持。
新增8项及XPC166/prepared4/existing18/core131/strictcore通过；最终CLI/build/
受测hash/Source0 smoke见[验证](validation/tunnel-window-storage-2026-10-15.md)。
其余H2container/outbound/private和fullfreshness继续，128MiB非完整heap/RSS；
L02不关闭、23/6/待收口、有效30与Q04全部排除保持，仅本地提交。

## 2026-10-15 L02 H2 stream state actual storage

stream/reset maps、四组membership及EOF/reset arrival队列按真实Layout创建前
准入，原known/local固定状态、最旧unknown淘汰/highwater不复用保持；原升序
stream选择移除collect/sort scratch。新quota路径保留已观察peer reset/END事实
和原quota source，private error/IO boxing仍MISSING。新增12项、XPC178/core131/
CLI504及strictcoreCLI/build/Source0manager/host5通过，2原ignored/22NOT_RUN，
见[验证](validation/tunnel-stream-state-storage-2026-10-15.md)。其它outbound/
initial/SDK/private与原fullfreshness继续；128MiB非完整heap/RSS，L02不关闭，
23/6/待收口、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-15 L02 H2/RSD sending storage

banked H2使用inline preface/control/ACK/headers/end和原DATA借用；banked RSD
host/三bootstrap从借用grammar先按真实wire Layout准入，不建AST/body/full临时
heap。Nonebank保留原编码/连续frame write，原wire/uuid/flags/window/deadline/
nativeerrors/strictEOF与最后owner收费保持。新增9项和XPC187/core131等通过，
完整门禁/受测binary/hash/Source0 smoke见
[验证](validation/tunnel-sending-storage-2026-10-15.md)。SDK/initial通用路径与
private runtime/error/allocator及fullfreshness继续，128MiB非完整heap/RSS；
L02不关闭，23/6/待收口、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-15 L02 borrowed vectored sending

banked H2将inline header与原payload slices一同交给底层vectored writer，partial
进度/标志和无支持时fallback保持；原native error pointer、标准WriteZero、
EOF与最后owner收费通过真实localhost TCP/duplex验证。新增3项、XPC190/
core131/CLI freshness13及strictcoreCLI/build/Source0 manager/host5通过，前一
提交9b703c3的完整CLI504证据准确分开，见
[验证](validation/tunnel-vectored-sending-2026-10-15.md)。其它SDK/initial/private/
fullfreshness继续，128MiB非完整heap/RSS，L02不关闭；23/6/待收口、有效30与
Q04全部排除不变，仅本地提交。

## 2026-10-15 L02 initial RSD scratch admission

initial direct/proxy沿用原reservation池，在attach前捕获；raw H2/bootstrap/
operation/borrowed arena创建前准入，旧空UID/零port/过滤规则与六semantic消息、
四mode及原期限保持。Nonebank/后续SDK/历史观察未改，公开目录持久buffer仍
MISSING，不能用设备sidecar覆盖into_rsd逃逸。XPC201及activation/proxy和无mdns
组合通过，最终门禁见[验证](validation/tunnel-initial-rsd-scratch-2026-10-15.md)。
128MiB非完整heap/RSS，原fullfreshness/private等继续；L02不关闭，23/6/待收口、
有效30与Q04全部排除不变，仅本地提交。

## 2026-10-15 L02 clear epoch failure/history race

原clear发布新epoch后再无条件清空failure/retiredcause，可能删除新epoch记录。
两项竞态已red复现；失败行保留原global owner、清理只移出失效epoch，退休
history在切换前取出；原即时取消/guard外析构/Pending与typed对象保持。最终
门禁见[验证](validation/tunnel-clear-epoch-2026-10-15.md)。仅修复原生命周期/退避
子项，完整freshness/private等仍继续；128MiB非完整heap/RSS，L02不关闭，
23/6/待收口、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 admitted Vec creation order

修正KnownVec/coordinator及八处直接backing先分配后收费的实际缺口，统一真实
Layout准入后分配；增长保留双份费用，canonical key先收费复制，paired/selected
局部Drop先释放数据再退费。完整tunnel518/0FAIL/2ignored、strictCLI/build/Source0
门禁通过，见[验证](validation/tunnel-owned-vector-creation-2026-10-16.md)。未收费
cleanup/history/HTTP/private与原完整freshness继续，128MiB非完整heap/RSS，
L02不关闭；23/6/待收口、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 native cleanup fixed selection

native stop/drain/caller-drop移除临时handle/key Vec，以永久issued/首次retired序号
冻结cutoff/count/capacity，晚退休/槽位复用不进入旧集合，原Key/namespace/立即
stop/绝对clock/joins保持；诊断两数组准入后复制所有phase的原owner。完整tunnel
527及最终相关1/strictCLI/build/Source0通过，见
[验证](validation/tunnel-native-cleanup-selection-2026-10-16.md)。其它history/HTTP/
private与原完整freshness继续，128MiB非完整heap/RSS，L02不关闭；23/6/待收口、
有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 prepaid immutable scan history

Session.begin在Source前独立预付/创建固定8根快照，abandon后冻结槽位顺序，
terminal只共享原Arc，零配额/closed-bank晚Drop保持原错误；census实际数组
另行准入，旧快照和bare original费用独立释放。完整tunnel530/strictCLI/build/
Source0通过，见[验证](validation/tunnel-history-snapshot-2026-10-16.md)。HTTP/
private/initial公开目录与原完整freshness继续；128MiB非完整heap/RSS，L02不
关闭，23/6/待收口、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 manager JSON wire owner

全部manager自有JSON handler改为同value计数/精确data准入/capped writer，grant
跟随最后Bytes frame/slice；固定诊断Source前预留，权限先于池，简单mutation
回复先构造。完整tunnel540/0FAIL/2ignored及最终Clippy/build/Source0通过，见
[验证](validation/tunnel-json-wire-owner-2026-10-16.md)（Replies局部largeErr例外
有理由/逐字节复用记录）。record/Value输入图/WS/auth/framework/private与原完整
freshness继续，128MiB非完整heap/RSS；L02不关闭，23/6/待收口、有效30与Q04
全部排除不变，仅本地提交。

## 2026-10-16 L02 manager record copy owners

原生record的独立文本先按精确Layout整笔准入再复制，PaidRecord最后副本退费；
本地list实际backing预付，start借用字段保留record费用。复制拒绝不再当404或
触发默认路由fallback，自动alive检查不复制输出。完整tunnel550/0FAIL/2ignored
及strictCLI/build/Source0通过，原native joins/strictEOF与精确独立退款已验，见
[验证](validation/tunnel-record-copy-owners-2026-10-16.md)。联邦Value/input与其它
HTTP/WS/private及原完整freshness继续，128MiB非完整heap/RSS；L02不关闭，
23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 fixed diagnostic hex storage

DiagnosticId/DiagnosticError内部两份hex String改为固定16字节值，保留原hash、
source/类别、4096字节/8层限制与fmt失败。core诊断allfeature7/minimal8及完整
tunnel550/core+CLI strict/build/Source0通过，见
[验证](validation/tunnel-diagnostic-hex-2026-10-16.md)。任意底层Error Display与
其它private/联邦Value/input/WS及原完整freshness继续，128MiB非完整heap/RSS；
L02不关闭，23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 local federation borrowed output

本地root listing直接借用PaidRecord，移除第二套Value/key/entries图；原地排序/
无索引合并保留同UID local-first、remote扩展与顺序、scope及旧对象字段字节。
完整tunnel555/0FAIL/2ignored、strictCLI/build/Source0通过，见
[验证](validation/tunnel-local-federation-view-2026-10-16.md)。remote图/input/
URL/join_all、WS/private和原完整freshness继续；128MiB非完整heap/RSS，L02
不关闭，23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 remote JSON owned destinations

上游自有body/JSON字符串及数组对象entry按实际Layout先准入，原Resource与
JSON错误类别保留；URL snapshot/source/index真实backing预付，借用overlay
保留扩展/顺序/target选择，原地sort/dedup避免倒序平方移动。完整tunnel565/
0FAIL/2ignored、上游6/strictCLI/build/Source0通过，见
[验证](validation/tunnel-remote-json-owners-2026-10-16.md)。decoder scratch/
join_all/RemoteTarget/private/WS及原完整freshness继续，128MiB非完整heap/RSS；
L02不关闭，23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 remote target text owner

target直接move原paid URL，interface/下一跳文本按精确Layout先准入再复制，
部分拒绝与manager/input销毁后的最后owner退款已验，原URL参数/选择保持。
完整tunnel568/0FAIL/2ignored、上游6/strictCLI/build/Source0通过，见
[验证](validation/tunnel-remote-target-owner-2026-10-16.md)。join_all/decoder/
URI/header/WS/private和原完整freshness继续，128MiB非完整heap/RSS；L02不
关闭，23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 federation future/result storage

两个联邦join_all以共用collector接线，原future实际Box及Slot/Output数组在
首次poll前预付；parent Context保留全并发/wait-all/注册顺序/原error和cancel
Drop，最后iterator/output独立持费。10项新增专项与完整tunnel578/0FAIL/
2原ignored、上游6/strictCLI/build/Source0通过，见
[验证](validation/tunnel-federation-join-storage-2026-10-16.md)。decoder scratch/
URI/header/WS/private及原完整freshness等继续；128MiB非完整heap/RSS，L02
不关闭，23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 upstream JSON string scratch

原锁定serde reader使用预付实际scratch Vec，并以逐字节同位置syntax副本保留
结构/number/depth和原typed错误/行列；同时存活syntax/scratch/Text全部持费，
最后树独立于body。完整tunnel582/0FAIL/2原ignored、上游6/strictCLI/build/
Source0通过，见[验证](validation/tunnel-json-string-scratch-2026-10-16.md)。
URI/header/registry/WS/private与原完整freshness等继续；128MiB非完整heap/RSS，
L02不关闭，23/6/待全审计、有效30与Q04全部排除不变，仅本地提交。

## 2026-10-16 L02 registered upstream storage

注册roots以实际paid Text数组替代BTreeSet私有node，初始roots和后续insert均
先准入目的地；晚期拒绝保持原表/pointer，duplicate零额外配额，移出Text持费至
最后owner。完整tunnel587/0FAIL/2原ignored、上游6/strictCLI/build/Source0通过，
见[验证](validation/tunnel-upstream-registry-storage-2026-10-16.md)。URI/header/
client/credentials/WS/private/manual副本与原完整freshness等继续；128MiB非
完整heap/RSS，L02不关闭，23/6/待全审计、有效30与Q04全部排除保持，仅本地提交。

## 2026-10-16 L02 shared route destination owners

首次host实际Layout创建前准入，publication/desired/live shortcut/自动恢复共享
原RetainedRoute Arc与文本，request filter借用；原published ceiling及native
joins费用保持，路由body独立最后owner退款经Weak已验。完整tunnel592/0FAIL/
2原ignored、上游6/strictCLI/build/Source0通过，见
[验证](validation/tunnel-shared-route-owners-2026-10-16.md)。observed/Source文本、
URI/header/WS/private及完整freshness等继续；128MiB非完整heap/RSS，L02不关闭，
23/6/待全审计、有效30与Q04全部排除保持，仅本地提交。

## 2026-10-16 L02 observed route destination owners

observed route共享已付Input原对象，无Input时按实际Layout先准入文本；保持
实际host与desired filter分离、原reconciliation和最后owner退款。完整tunnel
598/0FAIL/2原ignored、上游6/strictCLI/build/Source0通过，见
[验证](validation/tunnel-observed-route-owners-2026-10-16.md)。Source/candidate
创建、URI/header/WS/private及完整freshness审计继续；128MiB非完整heap/RSS，
L02不关闭，23/6/待全审计、有效30和Q04全部排除保持，仅本地提交。

## 2026-10-17 L02 upstream Authorization payload

原HTTP/WS request认证文本按实际Layout先准入，Bytes/HeaderValue共享原data，
最后header owner清零data后退款；目标/transport先验、原typed error/source和
敏感标志保持。完整tunnel603/0FAIL/2原ignored、上游7/strictCLI/build/Source0
通过，见[验证](validation/tunnel-authorization-payload-2026-10-17.md)。client
root/config/URI/framework/WS/private及完整freshness等继续，128MiB非完整
heap/RSS，L02不关闭，23/6/待全审计、有效30与Q04全部排除保持，仅本地提交。
