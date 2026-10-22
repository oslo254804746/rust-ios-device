# L02 desired key allocation before copy · 2026-10-22

Lifecycle::desired 原先先 udid.clone()，随后才按新 String.capacity() 申请
key 费用。原 KnownVec 虽已支付 DesiredRow backing，不能覆盖该独立 String
在拒绝前已发生的分配。本项在原行 reserve 后，先从同一 global bank 申请
exact_backing<u8>(udid.len())，再复制原 UTF-8 字节并让 String::from_utf8
消费同一 Vec。复制不增长 backing，原 native permit 跟随该 String 所在
DesiredRow 或显式转移的 key owner，String 实际释放后再退款。

没有改变 row/gate/epoch/desired snapshot、原顺序和部分列表回滚；row backing
与 copied key 仍独立计费，固定128MiB/4096/列表上限未变。原空/clear/terminal
列表零存储语义、manual-success owner/原 pointer/Source/期限保持。未新增
bank/unsafe 或 opaque allocation 估计。

原 exact listing 测试加强为精确两 backing 配额：实际 row 分配后只剩 key
费用，key 分配前全额扣零；对照真实 layout/align/地址，String 指针是同一
预付 backing，且独立于 canonical gate key。闭池零余额后通过 IntoIter 移出
原 String/permit，iterator 仅退 row 费用；String 实际 drop 后 key permit
仍扣，最后 permit 才退 key。所有实际 owner 结束后原65536测试池全额退款。

新增一个 UTF-8 key N−1 测试：仅允许 row backing 事件，拒绝的 key 没有
backing；原 canonical 指针/desired version/Stopped/gate count 保持，部分
row 费用精确返回。额度恢复后的真实新 key 内容/独立指针及最后全额退款
均验证。原第二 key 拒绝后的已建 batch 全部回滚、backing N−1、空列表与
clear/terminal 测试保留。只读 agent 源码审查未发现生产或测试缺陷，未编辑
仓库或运行 Cargo；审计在外部 desired-key-preallocation-audit.md/json。

round1 全11命令通过，全部 Cargo/build 由主控串行执行：

- CLI all-features/all-targets Clippy -D warnings。
- desired listing：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1796 filtered out; finished in 0.01s
- lifecycle过滤：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1772 filtered out; finished in 6.74s
- global metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1784 filtered out; finished in 0.05s
- success bookkeeping：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1790 filtered out; finished in 2.16s
- 完整CLI：test result: ok. 1788 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 237.56s
- build/fmt/diff、实际只读 manager1（Source/usbmux trap0）、host5 PASS /
  0 FAIL / 22 NOT_RUN，real_device_executed=false。

695冻结源码无漂移，2处差异仅CLI；Core/Cargo.lock 全量哈希与 a085f7e
状态相同，Core沿用846c533已验29门禁/tunnel159/owner42/无mDNS37/watch10/
无mDNS10，本轮未重跑Core。实际 host 与 verified-ios.exe SHA-256：
13bc6e75a8571cedb584b2c64b4fb263132c54ad5f8ea547f98463dee385d85b
命令日志、source delta/current/提交后rawGit proof及二进制归档：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-desired-key-preallocation\round1

scan外层frame/原marker、Core公开native Arc/Weak/watch、opaque error/report/
SDK/metadata、框架/HTTP/WS/credential/allocator/Waker/waiter及完整各模式
endpoint恢复/native exposure事务仍属原范围，未豁免。128MiB仍是已准入
已知对象池，非整个heap/RSS。未声称Miri/其它平台/真实设备验证。主项23
交付/6缺开发/L02待全审计、有效30/Q04全部排除保持。只本地提交并继续。
