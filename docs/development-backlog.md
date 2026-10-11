# 当前待开发清单

更新：2026-10-11。用户口径允许完整实现、仅等待运行验证的任务计开发完成；
代码、协议、生产接线、工具或收尾尚缺不能计完成。实际 NOT_RUN 不改成 PASS。
Q04及全部子项排除，有效任务分母保持30。

审查后承接原19项与L03，本轮A03共同owner/绑定/工具完整交付并通过实际gate：
21项可承接，8项仍缺开发，L02仍待完整核查。
这不等于原19项在Windows/其它平台全部重新验收通过。Viewer能力清单和
五项Windows core失败作为原工作项维护处理，不新增任务改变分母。
未计完成项按原范围继续；不因占位、拒绝或缺口说明升级计数。

| ID | 原范围中的剩余交付（修复后逐项复核） |
| --- | --- |
| Q02 | 已交付正确CLI、manifest、hash/失败/本轮清理；Cryptex与POSIX富clipboard observer已接线。私有MAC/altIRK、Display/HID、InstallCoordination、RSD模式关联4个精确生产receipt仍缺；应用/XCTest生产peer与阶段故障已执行，本轮最终制品复验继续。 |
| L01 | 剩余协议/服务和startup/upgrade/blocking/长流总期限、取消失效、原主/清理错误、native回收；局部SDK/restore/workflow修复不关闭全部profiles。 |
| L02 | 全局元数据/原error payload字节归因、freshness/刷新/复用/重启/manual epoch全规则；发布预算和epoch修复为子项，完整范围待核查。 |
| L04 | 已有生产native ICMP/AFC/Syslog、时窗/hash/前后分布和恢复证据；各负载全过程资源峰值/增长归因、其余范围与性能预算仍需补齐。真实TCP/DTX/平台工作负载不得由局部样本推导。 |
| F01 | USB枚举、选定TCP route和普通DDI目录检查已交付；remoted共存及目标DDI签名/manifest/设备接受仍缺，不能由Unknown推导Ok。 |
| F02 | 已交付独立启动清理owner、原DTX错误、部分产物/重跑历史及生产CLI阶段故障peer；受限crash收集、完整现代native/platform工作流与原验收继续，不能仅据legacy peer关闭。 |
| F04 | Runtime context销毁接线、Page/iframe、可信Input/截图、Fetch/下载、Memory/Performance真实映射和pinned实际客户端。 |
| F05 | PNG/Web/VNC共享源、输入/文本剪贴板owner和保护；原HEVC媒体解码、方向、有证据图像双向剪贴板、预算/退出及adapter流程。 |
| F06 | 有机型/固件/版本证据的预检和完整Update：recovery ramdisk、USB重举、nonce/TSS/IMG4、ASR/FDR/data-port传输/失败/清理；restored传输子项和拒绝不算完整执行器。 |

当前修复及实际验证见[修复记录](validation/review-fixes-2026-10-10.md)。
原定义保持在[工程路线图](engineering-roadmap-2026-09-30.md)。

A03开发状态关闭不表示全部真机/平台验收通过：Windows实际DLL foreign ABI与
installed Python30场景分别通过；原POSIX pthread finite-query、最低Python3.9、
其它平台制品及真实USB/Wi-Fi/设备服务继续NOT_RUN。legacy通用Lockdown和
不可物理取消的便捷文件loader按文档保留其兼容范围，不推导L01全部完成。
