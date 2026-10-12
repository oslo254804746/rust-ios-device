# 当前待开发清单

更新：2026-10-12。用户口径允许完整实现、仅等待运行验证的任务计开发完成；
代码、协议、生产接线、工具或收尾尚缺不能计完成。实际 NOT_RUN 不改成 PASS。
Q04及全部子项排除，有效任务分母保持30。

审查后承接原19项与L03，本轮A03共同owner/绑定/工具完整交付并通过实际gate：
L04原测量/资源归因/预算工具范围本轮交付，开发口径计入完成：
22项可承接，7项仍缺开发，L02仍待完整核查。
这不等于原19项在Windows/其它平台全部重新验收通过。Viewer能力清单和
五项Windows core失败作为原工作项维护处理，不新增任务改变分母。
未计完成项按原范围继续；不因占位、拒绝或缺口说明升级计数。

| ID | 原范围中的剩余交付（修复后逐项复核） |
| --- | --- |
| Q02 | 四个精确生产observer已接线：私有MAC/altIRK、Display/HID、InstallCoordination和RSD bootstrap模式。Windows watch隐藏private console/原Job/reader收尾工具已交付，9项通过（含实际CLI TLS startup取消）；原FUP-01–FUP-04交付边界继续逐项复核。完整Windows设备watch、paired-identity CLI/mDNS、kernel Display/HID、其它模式实际CLI及原物理/平台验收继续。 |
| L01 | 剩余协议/服务和startup/upgrade/blocking/长流总期限、取消失效、原主/清理错误、native回收；局部SDK/restore/workflow修复不关闭全部profiles。 |
| L02 | 全局元数据/原error payload字节归因、freshness/刷新/复用/重启/manual epoch全规则；发布预算和epoch修复为子项，完整范围待核查。 |
| F01 | USB/route/目录、remoted前后状态、挂载/签名观察与legacy/明确existing-RSD开发操作已接线，Doctor48项通过。legacy握手后raw DTX仅接受协商TLS1.2，TLS1.3明确Unsupported且无DTX；完整目标DDI manifest/签名证据和原平台/设备范围仍须核查，不由Unknown推导Ok。 |
| F02 | 限量crash收集、现代startup NativeScope、noWait事实receipt、WDA startup/ready owner与scoped forward已接线；修正WDA迟到成功丢owner的竞态，localhost实际CLI启动→TCP ready→单次转发→原PID/peer收尾5项通过。原DTX错误/部分产物/重跑/安装owner及实际legacy CLI故障peer已验。现代全流程和原平台/设备范围仍须核查，不据legacy子项关闭。 |
| F04 | Runtime context销毁接线、Page/iframe、可信Input/截图、Fetch/下载、Memory/Performance真实映射和pinned实际客户端。 |
| F05 | PNG/Web/VNC共享源、输入/文本剪贴板owner和保护；原HEVC媒体解码、方向、有证据图像双向剪贴板、预算/退出及adapter流程。 |
| F06 | 有机型/固件/版本证据的预检和完整Update：recovery ramdisk、USB重举、nonce/TSS/IMG4、ASR/FDR/data-port传输/失败/清理；restored传输子项和拒绝不算完整执行器。 |

当前修复及实际验证见[修复记录](validation/review-fixes-2026-10-10.md)。
本轮最小WDA交付与L01/L02/Q02/F01范围复核见[接力记录](validation/wda-continuation-2026-10-12.md)。
L04原范围交付与实测边界见[资源基线](validation/manager-resource-baseline-2026-10-11.md)：
同fixture前后比较、连续资源采样、独立默认堆归因与基线派生预算均已交付；
物理设备/其他平台/发布构建NOT_RUN，超出参考范围的性能值保持观察标志。
原定义保持在[工程路线图](engineering-roadmap-2026-09-30.md)。

A03开发状态关闭不表示全部真机/平台验收通过：Windows实际DLL foreign ABI与
installed Python30场景分别通过；原POSIX pthread finite-query、最低Python3.9、
其它平台制品及真实USB/Wi-Fi/设备服务继续NOT_RUN。legacy通用Lockdown和
不可物理取消的便捷文件loader按文档保留其兼容范围，不推导L01全部完成。
