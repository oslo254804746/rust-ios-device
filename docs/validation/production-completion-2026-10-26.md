# 剩余生产功能实施 — 2026-10-26

起点 `9f0e78a`，分支 `codex/review-fixes-20261010`，工作区干净。同一 checkout，仅本地提交。主控串行构建，所有设备协议验证使用离线/duplex/localhost；无真实设备、TSS或系统网络修改。

## 有限实施表

| 项目 | 本轮具体工作范围 | 验收出口 |
| --- | --- | --- |
| L01 | 实际 InstallationProxy 安装/升级进度 owner，GPX 请求与取消；复用已有 H2/DTX/AFC/Lockdown 等 profiles | 实际请求、部分发送取消、迟到消息/不可复用、主错误及清理 |
| F01 | 个性化/传统 DDI 版本策略、型号约束、目录错误与不完整报告 | 硬件/版本/组件/目录 fixture、无设备报告 |
| F02 | 现代启动已发送未确认 PID 的观察/收尾，公开可组合执行入口 | canonical/shim/DTX/AppService 假服务阶段及取消 |
| F04 | 根据 WebKit 实际协议推进上下文、输入、Fetch、指标 | 固定实际客户端经过 Rust 桥，无伪可信输入或常量指标 |
| F05 | Display 启停 owner、RGBA DIB、时间戳及生产慢消费者 | 原服务/任务收尾、真实像素与标准 RFB 流程 |
| F06 | 固定上游恢复路径逐部件核实并实现可独立执行部分 | 离线 IPSW、假 TSS/恢复服务；未实现的端点不得接通 CLI |

## F01 首包

个性化 DDI 不要求与设备同一 OS major/build，而按 board/chip/security identity 匹配；iOS17 之前不能选择此 profile。传统镜像检查 major/minor；若清单提供 SupportedProductTypes，额外检查目标 ProductType。目录枚举、entry/file_type 错误不再吞成 NoManifestFound；组件 metadata 的 PermissionDenied 不再被 is_file 提前吞掉。存在/摘要匹配仍不代表 Apple 签名已验证或设备已挂载。

协议来源：[固定 pymobiledevice3 mobile_image_mounter.py](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/services/mobile_image_mounter.py)，`request_personalization_manifest` 按 BoardId/ChipID 选择，不比较设备 build；`PERSONALIZED_IMAGE_MIN_VERSION` 为17。

执行 `cargo test -p ios-cli --all-features cmd::doctor:: -- --test-threads=4`：72 PASS / 0 FAIL / 1 原 child fixture ignored。日志 `.git/completion-doctor.log`。六主项计数暂不调整，后续包完成后统一登记。

## L01 安装与 GPX 生产消费者

新增 InstallationProxy `install_with_timeout`/`upgrade_with_timeout`，请求与所有进度回复共享一个总 clock。实际 apps install/upgrade 使用30秒服务建连、300秒操作总期限与 Ctrl-C；停止不声明设备安装已回滚，已发送后效果可能未知。部分发送/等待取消使原连接不可复用，迟到 Complete 不能被下一操作消费。暂存路径清理独立5秒，原操作和原清理错误均保留。

GPX CLI 现在使用 SimLocationClient 的逐点写入 owner（30秒请求），计划轨迹等待属于可取消 idle，Ctrl-C 关闭原客户端；此前已发送的位置可能仍有效。保留旧 raw API 的兼容行为。

Core installation30/0、simlocation11/0；CLI apps46/0、location定向通过，日志 `.git/completion-{install,gpx,apps-cli,location-cli}.log`。首次新fixture编译类型歧义已修，不作通过证据。既有H2/DTX/AFC等未变profiles复用交接证据；未凭这两个消费者声明全仓库L01完成。

## F02 执行入口与未确认启动

`ios-cli` 现在同时提供库，公开现有 TestWorkflowEngine/Args/RunOptions；binary仅调用同一run_cli。没有复制编排引擎，runtest/runxctest仍调用原run。XPC `call_observed` 在完整请求本地flush后记录submission；CoreDevice launch把此观察与已确认PID分开。现代启动在launch前保存同bundle旧PID基线，若已完整发送但PID未确认，cleanup在原服务reconnector上查询新进程，只对唯一、同bundle且不在旧基线的PID发送kill。未能查回不把关socket当成功，返回RunnerLaunchUnconfirmed或原重连/查询错误，继续testmanager/native收尾。

验证：runtest12/0，统一engine22/0，runxctest10/0，XPC18/0（含真实duplex完整发送后caller Drop、不可复用及peer EOF）。日志 `.git/completion-{workflow,engine,runxctest,xpc}.log`。公开API外部集成测试已注册，最终批次执行。尚未用这些结果声称现代canonical/shim/DTX/AppService全阶段假服务已经全验收。
