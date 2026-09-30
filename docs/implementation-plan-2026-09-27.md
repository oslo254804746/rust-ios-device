# 参考功能补齐实施计划

基于 [功能差异核对](reference-gap-audit-2026-09-27.md)，继续在
`feat/device-service-updates-20260927` 开发。已有首批功能和验证结果保留。
参考实现使用已 fetch 的远端 Git 对象，精确提交记录仍保存在仓库外；
不修改参考 checkout，不复制其源码，不访问真实设备执行安装、配对或恢复。

## 交付顺序与任务拆分

| 阶段 / 任务 | 交付范围 | 依赖 | 验收条件 |
| --- | --- | --- | --- |
| 第一阶段 A：RSD 身份与握手 | 跨进程稳定的主机 UUID；macOS remoted UUID 解析；现代主动握手；直连和 userspace proxy 使用同一身份，保留旧设备回退 | 无 | 合成握手验证 UUID 线类型与字段、重连身份一致、错误与超时有界；不破坏旧 bootstrap |
| 第一阶段 B：Cryptex DDI | Cryptex 服务查询和安装协议、DDI 个性化输入、iOS 27 自动选择、已有镜像冲突处理；按现有 CLI 习惯暴露 | 复用已有 XPC、mounter、TSS；公共注册由主代理合并 | 模拟服务验证命令、文件传输、票据、错误退出；旧系统继续走原 mounter；不能把普通 DDI 状态当作 Cryptex 就绪 |
| 第一阶段 C：Wi-Fi 身份识别 | authTag/altIRK 匹配、私有 MAC 场景、未知 UDID 候选配对记录；接入现有发现与连接流程 | 无；与 A 的公共入口由主代理协调 | 合成广播与固定算法向量、歧义处理、错误记录隔离和日志脱敏；只使用已有信任记录，不隐式创建配对 |
| 第一阶段 D：主机 doctor | 主机运行环境、usbmuxd 可达性、设备枚举和隧道能力诊断；JSON / 人类输出 | 无，由主代理执行 | 本机无设备也能形成报告；探测有期限；不可把“未探测”当作通过，不修改主机或设备配置 |
| 第二阶段 E：远端隧道 | tunneld `/connect` WebSocket 桥、目标隧道选择、远端 federation、hop budget | A、C 集成后 | 本地假服务覆盖双向数据、断开、背压、循环和超限；保留现有 TCP proxy |
| 第二阶段 F：CDP 兼容 | browser endpoint / Target 会话，随后 iframe / JSContext、输入截图、Fetch、Memory/Performance 各域 | 可独立开发；公共 CLI 注册串行合并 | 每个域独立交付；协议 fixture 与 Playwright/Puppeteer 客户端流程通过后才声明对应兼容 |
| 第二阶段 G：恢复协议 | preflight 请求生成、TSS batch、固件传输和新恢复消息 | B 的 TSS/文件传输接口稳定后 | 离线 fixture、假 TSS/恢复服务覆盖；保留现有 preflight-info；不在主机验证中刷机 |
| 第三阶段 H：查看器与剪贴板 | 先补 watch 自动重连，再补双向剪贴板回环抑制；解码后接 Web/VNC 查看器 | Display 生命周期已实现；依赖 A、B 可用 | 重连、重复事件、回环、解码预算、认证与 Origin 校验；现有原始 RTP API 继续可用 |
| 横向 I：平台差异核对 | legacy CopyDevices 断连、WebInspector 握手重试、native tunnel 退出、XDG 路径 | 随相应模块实施 | 先确认差距，再增加独立回归；未复现项保留“待核对”，不虚构修复 |

第二、三阶段是明确的后续交付，不因第一阶段完成而标记全部功能对齐。
对于协议证据不足的细节，实施者提交已确认字段、缺失证据和可执行下一步；
不以空实现或始终成功的占位分支完成任务。

## 第一阶段并行安排

| 执行者 | 所有权 | 当前状态 |
| --- | --- | --- |
| RSD 任务代理 | `xpc/rsd.rs`、新身份模块、`device/rsd_connect.rs` 及对应测试 | 已交付，集成检查通过 |
| Cryptex 任务代理 | 新 Cryptex 服务、`services/imagemounter/`、`cmd/ddi.rs` 及对应测试 | 已交付，集成检查通过 |
| Wi-Fi 任务代理 | `discovery.rs`、`device/discovery_match.rs`、配对记录解析与 Wi-Fi 连接相关代码及测试 | 已交付，集成检查通过 |
| 主代理 | doctor、公共 Cargo/导出/命令注册、文档、跨模块集成与验证 | 已完成 |

各任务代理在共享工作区内直接实现，不自行提交、切分支或推送；需要改动其他
任务所有权文件时先发消息协调。公共依赖、feature 和导出由主代理集中合并。
每个任务交付实际代码、协议测试、验证命令、已知限制和真机验收清单。

## 合并与验证

1. 模块实现后先运行相关 parser、codec、假服务和 CLI 参数测试。
2. 主代理审查超时／取消、内存预算、配对信息处理及旧接口兼容，更新功能文档。
3. 集成后运行 `cargo fmt --all -- --check`、
   `cargo check --workspace --all-targets --all-features` 和
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`。
4. 按仓库 CI 运行 `cargo test --workspace --exclude ios-py --all-features`，
   Python host 测试使用根目录 `.venv` 单独运行；若影响绑定，再执行 maturin
   构建／导入检查。环境配置见 [Python 构建说明](python-binding.md)。
5. 验证通过的阶段提交并推送到当前远端分支，报告准确的已实现和待实现项。

真机验收另列：iOS 27.2 多进程 RSD 重连、macOS remoted 共存、iOS 27 Cryptex
安装与服务可用性、私有 MAC 的仅 Wi-Fi 发现，以及之前待验的媒体／剪贴板行为。
主机测试通过不替代这些真机证据。

## 第一阶段交付结果（2026-09-30）

A、B、C 和主机范围的 D 已完成。RSD 的 macOS 身份解析、主动握手及回退、
Cryptex 的 nonce/TSS/五文件上传、Wi-Fi 的固定密码算法向量与候选校验、doctor
的期限／响应预算／输出脱敏均有主机测试。后续 E–I 仍按上表保留，未开始实施。

- 格式检查、workspace 全目标／全 feature 编译、严格 Clippy：通过。
- `cargo test --workspace --exclude ios-py --all-features`：1508 通过，
  0 失败，1 项既有高成本 PBKDF2 测试忽略。
- 根目录 `.venv` 的 Python host 测试：6 通过；maturin abi3 wheel 重建和
  editable 安装、导入、公开 API 与无效参数合同：通过。
- `cargo test --workspace --all-features` 的 PyO3 `extension-module` 测试
  二进制仍缺 Python 链接符号；采用仓库 CI 的上述拆分方式验证。
- base、mdns、cryptex 最小 feature 库编译：通过；裁剪组合仍有未使用代码告警，
  全 feature 严格 Clippy 无源码告警。
- CLI 无设备场景的 JSON／人类诊断、退出码、期限和新命令帮助：通过。

本阶段没有连接真机、发起配对、安装镜像或调用真实 Apple TSS。
剩余覆盖与验证边界已更新到 [功能差异核对](reference-gap-audit-2026-09-27.md)，
具体用法见 [Cryptex DDI](cryptex-ddi.md)、[Wi-Fi 发现](wifi-discovery.md)
及 [主机诊断](troubleshooting.md#host-diagnostic-report)。
