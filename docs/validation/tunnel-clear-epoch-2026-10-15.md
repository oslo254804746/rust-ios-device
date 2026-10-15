# L02 clear failure/history epoch cutover · 2026-10-15

原 clear() 在移走 publication 并发布新全局 epoch 后，先清理旧 native/retry，
随后无条件 failed.take_all()/clear_retired_causes()。这个间隔内新 epoch 可以
正确提交新的失败行，或在 Source 返回/stop 后产生新的退休错误；旧 clear 随后
会误删这些新记录，丢失失败次数、退避依据和原错误持有者。

修复保留原有即时取消时点：先切换 epoch 并移走旧 publication，再等待失败表。
每个提交的失败行在原 Ticket fence 内保留原 global epoch Arc；其 inline 字段
随原 Node 的真实 size 在 Source 前准入，不创建新 epoch 或额外 heap owner。
清理取得失败表后，在当前 global watch read fence 内移出过期行，保留当前 epoch
的行。这样后续 clear 也能回收被中途取消的旧 clear 遗留的更早行。新 epoch 的
退避判定、计数和 prune 同时识别行的 global owner，不继承旧 epoch 的失败历史；
原 per-device stop/manual、USB/paired/未完成 discovery 规则保持。

切换前移出既有 Retired receipt，保留 Pending Source 节点。所有表/watch/retry
锁释放后，才析构原错误及费用所有者；native/retry 取消仍按 immutable old-global
identity 匹配。retry mutex 不跨 lifecycle watch write，原 Ticket→retry 顺序保持。
没有创建替代 NativeLease 或延长连接/收尾期限。

red2 在原实现上确定复现两项竞态：replacement epoch 的失败行被删除，以及
新退休 IO 原对象被销毁。曾尝试在切换前等待失败表，但完整 green 回归的既有
retention 用例指出这会延后即时取消（508PASS/1FAIL/2ignored），最终改为 epoch
归属过滤。red2 中要求延后 epoch 的新增断言随之纠正，原即时取消测试保持。
此前 red 仅是新测试的编译失败，不冒充竞态复现；全部失败日志保留。cfg(test)
seam 在 epoch 切换/释放 publication 锁后插入新操作，不依赖 sleep 或概率调度。

最终 green3 另覆盖 Pending Source 原预留节点跨 clear 存活、原错误到达后退休，
clear 等待 publication 锁被取消后原 epoch/失败版本和费用不变，以及下次 clear
回收此前已切换 epoch 却被取消的清理遗留行。测试核对
新失败行原 version/slot/count/identity、原 typed IO 地址、最后费用归还，以及
payload Drop 能立即获得两张表写锁。仅 cfgtest FakeSource，Source 调用为 0。

本次只改 CLI build-source。source-delta.json 核对 ios-core/Cargo 全部
哈希与前一提交 709db59 一致，复用该提交的 core XPC201/tunnel131/activation5/
proxy3/no-mdns4/prepared4/existing18 证据。前一提交的完整 CLI1480/13 原 ignored
证据属于其原 HEAD；本次以完整 tunnel 组及严格 CLI/build/只读 smoke 验证。
green2 编译时遗漏的三处私有测试构造字段已补齐，日志保留；green3 的完整
tunnel 组 510PASS/0FAIL/2 原 ignored 通过，包含新增六项及原即时取消回归。
严格 CLI all-feature/all-target Clippy、build/fmt/diff 通过。只读 manager1 通过，
Source/usbmux trap0；host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
冻结 603 个 build-source，源码无漂移。受测 CLI SHA-256：
`afe460de80222a341bc1fa4bd66794cef6c275dfb4cb982ecc754d5d4e95169b`。
受测 binary、完整日志、source-delta、当前及提交后 raw Git blob proof 均在 green3。

证据：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-clear-epoch`。

该实际竞态修复属于原 L02 生命周期/退避范围，不能替代完整 freshness、
initial 公开目录/SDK/private allocation 合同或未执行设备/平台验收。128MiB 非完整
manager heap/RSS；L02 保持待全审计，23/6/待收口、有效分母 30、Q04 全部排除
不变。只本地提交，无真实设备操作或 push/发布/部署。
