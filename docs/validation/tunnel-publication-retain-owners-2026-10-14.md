# L02 publication pruning 原 owner · 2026-10-14

通用 Epoch::retain 不再复制待删除 key 或创建未收费 key/removed Vec。新路径
先核对原 epoch/绝对 clock、同池准入真实临时 KnownVec backing，再移动原
HashMap entry。predicate 在 watch guard 外只执行一次；keep 行重插原 entry，
removed 行保留原 String/值/费用。late clock、stale epoch 或 predicate unwind
均由原 owner guard 恢复，恢复不申请额度、不复制 key 或 Drop 原 native lease。
原 map 的 allocated backing 和容量保留，不承诺 inline value 地址或遍历顺序。

四条生产路径已接线：初始权限 pruning/capture、USB ListError、USB attachment/
local liveness，以及完整 paired view。原 complete/incomplete、manual、exact
scan-start publication Arc 和 UID/native-ID/host 条件保持。成功 removal 返回持费
owner，caller 先释放 table guard 再销毁原 key/value/native/费用；初始 capture
失败也按已成功的 pruning commit 收尾，不撤回该独立提交。

新增7项覆盖真实 backing 与最后 owner、一字节拒绝前 predicate0、actual late
clock/零余量恢复、stale/unwind 恢复、table guard 外析构，以及实际 userspace
native runtime 的 late-clock/零额度拒绝后原 key/publication/Arc 保持。原 runtime
最后由精确原 handle shutdown/join，严格 peer 零字节 EOF 和任务 baseline 恢复
保留，没有用 Drop/RST/flush 认证设备 ACK。

首轮468 PASS / 1 FAIL / 2原ignored：新增上游临时 peak 抢先拒绝，原 pending
一字节 fixture 尚未到达目标。现在 cfg(test) hook 在真实 USB snapshot 前、上游
scratch 已释放后控制额度；原 USB1/paired0、原 epoch/publication、精确退款断言
保留。initial capture 的一字节 fixture 同样计入其仍 live 的实际临时费用。
失败日志保留，没有仅凭 Source0/旧 Arc 推导目标准入已执行。

最终冻结563个 build-source：整组 tunnel469 PASS / 0 FAIL / 2原ignored；预算
16项、严格 CLI Clippy/build/fmt/diff、实际只读 manager1全部通过。Source/usbmux
trap0；host5 PASS / 22 NOT_RUN / 0 FAIL，real_device_executed=false。source/raw
Git blob、CLI/hash 与本地提交关联保存在外部 proof，设备/其它平台/SDK/feature/
MSRV 保持原证据或 NOT_RUN。

本项支付临时实际 backing，并保存已有 row/key/native owner。publication 原
HashMap bucket/control backing、原 dyn Error/std IO/anyhow/Arc/native 创建合同、
nonempty manual cleanup 晚准入、runtime存储及完整 freshness/manual epoch仍需
交付。128MiB不是完整 manager heap/RSS 上限，L02 主项未关闭；账本23交付/
6缺开发/L02待收口、有效30及 Q04 全部排除保持。

证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\13\01a1346f-a4f5-7e60-aaad-e51c0c37fbe8\repair\l02-publication-retain`。
原失败在父目录，完整成功、冻结/CLI与最终 proof 在 `final`。只本地提交，无
设备发现、恢复、配对、安装、push、发布或部署。
