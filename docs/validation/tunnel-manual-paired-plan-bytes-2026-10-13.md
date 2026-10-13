# L02 manual pending/planned 容器归属 · 2026-10-13

本项收口暂停时未提交的 pending_mobdev2/planned 改动，不关闭 L02 主项。
原 pending Vec 复制 UID、ticket、host filter，planned Vec 再保留这些副本；
两个 backing 未收费。现在两个实际 KnownVec backing 都接入现有共享池，
元素借用已准入 desired listing 的原 UID/ticket/host filter，只有最终准备
candidate Input 时才建立已经有 owner 的 UID/实际目标 host buffer。

pending 在 paired browse 前准入，borrow 的生命周期要求原 desired owner
继续存活。planned 在整个 paired batch 准备前准入，失败返回 Incomplete，
释放部分 plan 与 raw snapshot charges，保留已准备 USB window、原 manual
intent/发布及 unknown paired 状态；不因 quota 拒绝建立新 automatic ticket
前缀。索引匹配继续取原第一条匹配 UID/原 host pin，hostless selector 重新
发现实际 host。IntoIter 持有原 backing 费用直到其最后 owner 释放。

暂停前启动的整组门禁已实际完成：tunnel442 PASS / 0 FAIL / 2原ignored。
随后新增两个一字节边界测试：pending 拒绝时尚未 paired browse，无额外连接，
原 intent version/发布 Arc 保持且费用退款；paired raw 已准入但 planned
拒绝时标记 Incomplete，保留原 manual 和已发布 USB 两个 Arc，无额外Source
连接。针对性3 PASS包含两个新增与一个原 fallback tail 验证。生产代码未在
整组门禁之后改动，其余原442项按影响复用，不称单次444整组执行。
严格CLI Clippy、CLI build、fmt/diff通过；实际只读manager1通过，Source
连接为0；主机runner5 PASS / 22 NOT_RUN / 0 FAIL，real_device_executed=false。
其它未变Core/F04/SDK/feature/MSRV与平台范围按影响复用或保持NOT_RUN。

原 error/native/runtime/private allocator 创建合同、seen/observed、失败表
额外 key/route/backing 归属及完整 freshness/reuse/restart/manual epoch继续。
原 candidate Input 已为一个 outcome UID clone预留费用；本项不推导全部
私有或其它副本已计量，不声称整个manager heap/RSS上限。
账本23开发交付/6缺开发/L02待收口，有效30，Q04全部子项排除。
无真实设备发现、配对、安装或恢复操作。

原始日志与最终555个build-source冻结、提交关联：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-manual-paired-plan`，
最终门禁及proof在`final`，暂停前原整组日志保留于父目录。
