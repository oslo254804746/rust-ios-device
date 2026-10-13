# L02 扫描起始 publication 快照字节归属 · 2026-10-13

生产 refresh_scan 的起始 publication 索引原先将每个 table key 复制到未收费
HashMap。现在同步捕获到 InitialPublications：实际 KnownVec capacity × Row
size 与每个复制 String 的实际 capacity 在已有共享池准入。行中的 key permit
随快照存活，publication 继续使用相同 Arc 身份；不更换原发布 owner。

捕获在 USB/paired Source 之前完成，不持全局表锁执行网络等待。任意行拒绝
时，未完成快照及其已准入 backing/key 一并释放。空快照不分配 backing。
私有 rows 不向 caller 转移 key，lookup 只借用原 publication Arc；最后快照
owner 释放才退款。完整快照保留 ListError、USB detach 和完整 paired view
对 exact initial publication 的原有判断，不扩大旧发现对新发布的删除权限。

已新增两项直接 owner 测试：短 key 的原 reserve 不被替换，复制 buffer 与真实
backing 费用保持到最后共享快照释放；一字节不足时拒绝并全额退款，原 key/Arc
保持。另增加生产 preflight 拒绝测试，实际测得 desired listing 费用后，仅留下
scan capsule + listing + snapshot 一字节不足的额度，验证零 USB/paired Source
调用和原 publication 身份。Root 串行门禁436 PASS / 0 FAIL / 2既有ignored，
包含新增3项及原错误、deadline、取消、native/task join、严格EOF和回收断言。
原tinyquota夹具与断言未修改。严格CLI Clippy、CLI build、fmt/diff通过；实际
只读manager1通过且Source连接为0；主机runner5 PASS / 22 NOT_RUN / 0 FAIL，
real_device_executed=false。Core/F04及其它未变门禁按源码影响复用前一提交证据，
未称全Rust/SDK/feature/MSRV/其它平台重跑。

这项没有覆盖其它 scan HashMap/HashSet/Vec、success/failure key 副本、任意
error/Arc/allocator/native/runtime 私有分配或完整 freshness/reuse/restart/manual
epoch。它不是整个 manager heap 或 RSS 上限，不关闭 L02；账本仍23交付、
6缺开发、L02待完整收口，有效30，Q04及全部子项排除。无真实设备发现或操作。

源码冻结、完整日志及失败制品保存于
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-scan-publications`。
553个build-source的原Git blob、工作源码、二进制与精确提交关联保存为
`postcommit-proof.json`，旧已验二进制和历史制品保持。
