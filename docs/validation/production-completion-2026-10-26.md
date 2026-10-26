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
