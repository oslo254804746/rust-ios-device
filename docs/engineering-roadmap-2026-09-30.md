# 项目工程回顾与后续实施计划

日期：2026-09-30；代码基线：`572c297`。

状态：首类 B01–B04 的可执行主机范围已交付；B01/B02 完成，B03 待跨平台验收，
B04 保留平台和 registry 发布条件。接续第二类，实际证据见第 14 节。

## 1. 目标与执行范围

本计划承接本次工程回顾中已认可的建议，把项目的后续工作组织成可独立交付、
可验证、可追踪的任务。总体目标是形成可靠的跨平台 iOS 自动化与诊断基础设施，
使功能覆盖、接口兼容、运行稳定性和验证证据同步增长。

计划编写阶段的交付是本文；用户已于 2026-09-30 授权完整实施与本地阶段提交。
实施按依赖持续推进可执行任务；缺少设备、协议证据
或平台条件的任务记录具体缺口，同时继续不依赖这些条件的工作。

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
只有所列交付物和验收条件满足后才能标记完成。主机实现完成但真机验收缺条件时，
分别记录两个结果，任务保持待条件或待验收。

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
| B03 | 统一本地与 CI 验证入口 | P1 | B01、B02 | 待验收 |
| B04 | 补齐发布门禁与制品验证 | P1 | B03 | 待条件 |
| Q01 | 建立能力成熟度与验证矩阵 | P2 | 无 | 完成 |
| Q02 | 建立回归工具与代表性真机流程 | P2 | Q01、S04 | 待开始 |
| Q03 | 建立跨平台路径与输出失败合同 | P2 | B03 | 待验收：Linux 主机完成；其他平台/UNC 待条件 |
| Q04 | 增加协议 fuzz 与高成本专项检查 | P2 | B03 | 待开始 |
| S01 | 管理器认证与设备、端口授权 | P1 | 无 | 待开始 |
| S02 | HTTP 状态变更及浏览器访问边界 | P1 | S01 | 待开始 |
| S03 | 远端受保护传输与联邦凭据隔离 | P1 | S01、S02 | 待开始 |
| S04 | 统一敏感字段和日志脱敏 | P1 | 无 | 进行中 |
| R01 | 拆分备份协议、路径与会话职责 | P2 | B03、Q03 | 待开始 |
| R02 | 拆分 WebInspector 命令与服务职责 | P2 | B03 | 待开始 |
| R03 | 收敛 device 模块的文本包含结构 | P2 | B03 | 待开始 |
| R04 | 统一 plist/JSON 转换与输出策略 | P2 | A01 | 待开始 |
| A01 | 建立 Rust、CLI、C、Python 兼容规则 | P2 | B03 | 待开始 |
| A02 | 建立稳定错误分类与输出合同 | P2 | A01、R04 | 待开始 |
| A03 | 扩展常用任务的 Python/C SDK | P3 | A01、A02 | 待开始 |
| L01 | 统一期限、取消和连接失效规则 | P2 | B03、R03 | 待开始 |
| L02 | 隔离多设备生命周期与连接并发 | P2 | L01、S01 | 待开始 |
| L03 | 增加隧道、剪贴板自动重连 | P2 | L01、Q01 | 待开始 |
| L04 | 建立性能、资源与长时间运行基线 | P2 | L02、L03 | 待开始 |
| F01 | 扩展 doctor 的设备能力诊断 | P3 | Q01、L01、S04 | 待开始 |
| F02 | 完成应用、测试、日志与结果工作流 | P3 | F01、A02、L03 | 待开始 |
| F03 | 建立经过 Rust 桥的真实 CDP 客户端测试 | P3 | R02、S02、B03 | 待开始 |
| F04 | 按协议域完成 CDP 兼容流程 | P3 | F03、Q02 | 待开始 |
| F05 | 完成媒体查看器、VNC 与双向剪贴板 | P3 | S02、S03、L03、Q02 | 待开始 |
| F06 | 完成恢复预检入口与实验恢复执行流程 | P3 | A02、L01、Q02、Q04 | 待开始 |
| D01 | 维护文档、变更记录与交付规则 | P2 | 首批整改依赖 B01、B02、Q01；随后持续执行 | 进行中 |
| D02 | 建立质量指标与 1.0 发布准备标准 | P2 | 指标先随 Q01 建立；发布准备依赖 B04、A01、A02、L04、F02 | 进行中 |

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

### Q04：协议 fuzz 与高成本检查

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

全部实施完成的判定是：31 个主任务及其列出的功能子任务均满足各自验收，
必要真实证据已有索引，当前文档与发布合同一致。条件不足的任务可以继续排队，
但不能从清单中静默删除或被其他测试结果代替。后续范围调整须明确修改对应任务、
原因和验收边界，保留既有交付记录。

## 12. 建议的首轮实施顺序

1. 完成 B01、B02，先关闭已复现的最小接入与默认文档问题。
2. 完成 B03、B04，统一验证与发布门禁，落实 D01 首批文档整改。
3. 建立 Q01，并推进 S04、Q03；建立可靠的能力、输出和证据基线。
4. 完成 S01–S03，随后验收远端管理与浏览器入口的安全合同。
5. 推进 Q02、Q04；独立开展 A01 和 R01–R03，再落实 R04、A02。
6. 完成 L01–L04，以模拟多设备和代表性真机结果检验稳定性。
7. 按实际依赖推进 A03、F01–F06，逐子任务交付，持续更新 D01–D02。

实施状态以第 3 节总览和第 14 节交付记录为准；历史基线不作为新实现的通过证据。

## 13. 目标模式执行与大类提交规则

用户已要求在独立新会话中启动本计划：模型使用 `gpt-6.1-sol`，推理等级为
极高（`xhigh`），以持久化目标模式持续推进。每完成一个大类，必须完成验收、
本地 Git 提交和计划更新，再开始下一大类。目标不预设 token 或时间预算。

大类按可交付工作包组织，以解决字母分组之间的交叉依赖：

| 顺序 | 大类 | 对应任务 |
| --- | --- | --- |
| 1 | 构建、接入与发布 | B01–B04，D01 首批相关文档整改 |
| 2 | 能力证据与管理面安全 | Q01–Q03、S01–S04 |
| 3 | 模块结构与公共契约 | R01–R04、A01–A02 |
| 4 | 生命周期、性能与协议专项检查 | L01–L04、Q04 |
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

只有 31 个主任务及所有列出的子任务实际达到验收条件，持久化目标才能标记完成。
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
