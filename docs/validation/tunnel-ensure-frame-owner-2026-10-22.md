# L02 complete explicit-start frame owner · 2026-10-22

ensure_tunnel_with_intent 的原具体 async frame 会在设备协调/serial等待、
live RSD refresh、manual commit 与默认 paired fallback 期间持有 Ticket、
desired观察及原错误。该 frame 的实际存储此前未独立准入。本项保持原
selectors→manual exact UDID→ConnectionDeadline.new 同步前缀，随后从原
global bank 精确申请实际 ensure_admitted F 的 backing，并复用既有
PaidFuture Box/pin/最后真实释放后退款。第一次 paid F poll 才执行原
deadline.run(lifecycle.ticket)。从该句到最后 await 与4ff68c7原正文逐字
比较一致；没有新terminal检查、时钟重启、Source/fallback分支或任务。

新增F拒绝是明确的 pre-Ticket/pre-Source ResourceLimit，包括已准备的零
期限或终止状态。原已准入路径仍由原 deadline.run/Ticket 的先后逻辑产生
native Elapsed/OperationCancelled；已获取原Source/IO/elapsed不被替代。
原 absolute deadline、authority/epoch/manual rollback、重试计数与原
route选择和默认fallback guard保持。没有新unsafe、bank或固定限额扩张。

新增6测试验证：

- default/usbmux/USB/WiFi四route×normal/zero deadline，各F N−1，实际
  coordinator锁持有时立即拒绝；零backing、Ticket namespace/native/retry/
  Source/paired工作和精确原池退款。
- 原deadline overflow、manual empty exact selector、超长selector前缀及
  有效前缀closed0矩阵；原固定决定与分类在F前保留。
- 两个具体F的实际layout/地址/逐次整笔扣费；coordinator锁前分别捕获
  global epoch A/B并clear，闭池0时两个旧epoch由各实际F继续持有；实际
  Drop第一份只结束A及一份F费用，第二份结束B及另一份费用。此用例验证
  pre-coordinator的global引用，未声称它验证不同device/desired epochs。
- actual F allocation hook延迟60ms，原20ms连接期限native Elapsed，
  coordinator/Source均未执行，证明未在分配后重启时钟。
- 实际pending FakeSource，闭池caller Drop使原Source退出一次；预付的
  canonical gate/table保持，实际F及其临时owner归还，原清理与最终退款。
- localhost POST start-tunnel，Replies2048+F N−1、实际coordinator锁保持，
  原502 resource_limit在协调/Source/默认paired fallback前返回，实际最后
  HTTP/router owner结束后全额退款。

三处旧夹具保持原拒绝点：route/lifetime global短额度仅加实际F费，独立
publication池短额度不变；live record-copy保留直接get/list零额度，随后
只从原held permit释放exact F费，再要求原typed CopyAdmission。Source1/
paired0/原live device/drop0保持。direct ticket→connect_and_store 的原native
metadata夹具不经过ensure，不加F；已完成HTTP setup后才施加的scan额度、
Source内动态余额压力也不重复加费。

只读设计审计核对原正文、prefix和具体夹具（外部ensure-frame-owner-design-
audit.md/json，15源码指纹），未编辑仓库/运行Cargo。该agent后续触及额度
限制，未将它的退出当作完整新增测试审查；主控独立完成实现检查与所有
运行门禁。R1首个strict测试编译因新fixture对无Debug的ScopeDenied使用
unwrap失败，仅改测试的错误分支，不给生产类型新增Debug/allow。

round2 全17命令通过，全部Cargo/build主控串行：

- ensure-frame过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1806 filtered out; finished in 0.09s
- manual-deadline过滤：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1799 filtered out; finished in 2.88s
- manual-admission过滤：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1799 filtered out; finished in 0.80s
- intent过滤：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1801 filtered out; finished in 1.70s
- upgrade-epoch过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1806 filtered out; finished in 0.32s
- record-owner过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1807 filtered out; finished in 0.02s
- native-metadata过滤：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1805 filtered out; finished in 0.03s
- freshness过滤：test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 1790 filtered out; finished in 1.00s
- shutdown-admission过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1804 filtered out; finished in 2.06s
- scan-frame过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1807 filtered out; finished in 0.14s
- cli-full过滤：test result: ok. 1799 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 239.08s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。697冻结源码无漂移、6处差异仅CLI，原ensure
正文一致性另有guard；Core/Cargo.lock与4ff68c7全量hash相同，Core沿用
846c533已验29门禁/tunnel159/owner42/无mDNS37/watch10/无mDNS10，未声称重跑。
实际host及verified-ios.exe SHA-256：
d7e6b6bae5ddbade71861ddb4c47ca3f73cd53d6ac665ded4a92df7031b20ede
命令日志、source delta/current/提交后rawGit proof和实际二进制归档：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-ensure-frame-owner\round2

F费只覆盖该实际allocation，不代替返回record/error或opaque父HTTP/
Principal/task/authority深层堆。marker自身根、公开Core Arc/Weak/watch、
SDK/metadata/error/config/credential/transport/allocator/Waker/waiter及完整
各模式endpoint恢复/native exposure事务仍属原范围。没有有限marker arena
容量结论或整个heap/RSS/其它平台/Miri/真机声明。128MiB仍为已准入已知对象
池，4096及其它固定限额保持。L02仍待全审计，23交付/6缺开发/L02待核、
有效30/Q04全部排除不变。仅本地提交并继续原七主项。
