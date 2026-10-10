# 当前待开发清单

更新：2026-10-10。用户口径允许完整实现、仅等待运行验证的任务计开发完成；
代码、协议、生产接线、工具或收尾尚缺不能计完成。实际 NOT_RUN 不改成 PASS。
Q04及全部子项排除，有效任务分母保持30。

审查后承接原19项与新增L03：20项可承接，9项仍缺开发，L02仍待完整核查。
这不等于原19项在Windows/其它平台全部重新验收通过。Viewer能力清单和
五项Windows core失败作为原工作项维护处理，不新增任务改变分母。
修复仍在进行，集中验收前不因静态代码或子agent自述升级计数。

| ID | 原范围中的剩余交付（修复后逐项复核） |
| --- | --- |
| Q02 | 正确CLI、fixture manifest、逐矩阵流程/观察与授权分离、预期失败/hash、本轮清理；复杂协议用例不能用info/list替代。 |
| A03 | apps/AFC/logs与有限query共用checked owner、总预算/输出/并发、取消与close/join；公开aclose和installed验证、C linked服务及平台边界。 |
| L01 | 剩余协议/服务和startup/upgrade/blocking/长流总期限、取消失效、原主/清理错误、native回收；局部SDK/restore/workflow修复不关闭全部profiles。 |
| L02 | 全局元数据/原error payload字节归因、freshness/刷新/复用/重启/manual epoch全规则；发布预算和epoch修复为子项，完整范围待核查。 |
| L04 | 生产native/AFC/Syslog/DTX负载、持续时窗、资源采样、收尾/hash/p50/p95及同fixture前后比较；平台/真机结果独立保留。 |
| F01 | 平台USB/选定route/remoted真实观察和目标DDI可用性；Unknown不能报告Ok，目录不能替代签名/manifest/设备接受。 |
| F02 | 全阶段owner、真实bundle/安装所有权、安装/配置/报告/取消失败收尾、日志join、重跑历史与整流程期限、每阶段fixture。 |
| F04 | Runtime context销毁接线、Page/iframe、可信Input/截图、Fetch/下载、Memory/Performance真实映射和pinned实际客户端。 |
| F05 | PNG/Web/VNC共享源、输入/文本剪贴板owner和保护；原HEVC媒体解码、方向、有证据图像双向剪贴板、预算/退出及adapter流程。 |
| F06 | 有机型/固件/版本证据的预检和完整Update：recovery ramdisk、USB重举、nonce/TSS/IMG4、ASR/FDR/data-port传输/失败/清理；restored传输子项和拒绝不算完整执行器。 |

当前修复及实际验证见[修复记录](validation/review-fixes-2026-10-10.md)。
原定义保持在[工程路线图](engineering-roadmap-2026-09-30.md)。
