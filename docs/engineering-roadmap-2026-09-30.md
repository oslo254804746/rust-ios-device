# 项目工程回顾与后续实施计划

日期：2026-09-30；代码基线：`572c297`。

状态：第一、二类的当前 Linux 主机范围已交付；B01/B02、Q01、S04 完成，
其余平台、部署、真机及 fixture 子项保留待条件/待验收。接续第三类，实际证据见第 14 节。

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
| Q02 | 建立回归工具与代表性真机流程 | P2 | Q01、S04 | 待条件：Linux 统一捕获/owned cleanup、27 行风险矩阵及 FUP 文档复核已交付；设备 adapters/真实工作流、Windows/UNC、历史报告勘误待条件 |
| Q03 | 建立跨平台路径与输出失败合同 | P2 | B03 | 待验收：Linux 主机完成；其他平台/UNC 待条件 |
| Q04 | 增加协议 fuzz 与高成本专项检查 | P2 | B03 | 待开始 |
| S01 | 管理器认证与设备、端口授权 | P1 | 无 | 待验收：manager/CLI/doctor/debug 与 S03 独立联邦凭据主机接入已交付；平台文件/ACL、真机转发/会话撤销待条件 |
| S02 | HTTP 状态变更及浏览器访问边界 | P1 | S01 | 待验收：既有入口 Linux 方法/Host/Origin/CORS、单次票据及实际浏览器主机验收已交付；F05 新入口、跨平台与真机工作流待条件 |
| S03 | 远端受保护传输与联邦凭据隔离 | P1 | S01、S02 | 待验收：Linux HTTPS/WSS、独立凭据/CA、重定向隔离与两跳主机范围已交付；平台文件/部署/真实设备待条件 |
| S04 | 统一敏感字段和日志脱敏 | P1 | 无 | 完成：自有输出边界及 CLI 依赖日志来源已验收；新增接口继续沿用 |
| R01 | 拆分备份协议、路径与会话职责 | P2 | B03、Q03 | 完成（主机边界与回归验收） |
| R02 | 拆分 WebInspector 命令与服务职责 | P2 | B03 | 完成（主机边界验收） |
| R03 | 收敛 device 模块的文本包含结构 | P2 | B03 | 完成 |
| R04 | 统一 plist/JSON 转换与输出策略 | P2 | A01 | 完成（主机编码与stdout合同验收） |
| A01 | 建立 Rust、CLI、C、Python 兼容规则 | P2 | B03 | 完成：候选合同、版本/迁移与现有 public 字段/enum 风险登记；Linux 代表性下游/CLI/C 与 Python 3.9/3.12/3.13 通过，设备成熟度不因此升级 |
| A02 | 建立稳定错误分类与输出合同 | P2 | A01、R04 | 进行中 |
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
