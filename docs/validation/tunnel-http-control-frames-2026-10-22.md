# L02 HTTP control future owners · 2026-10-22

HTTP DELETE tunnel 的原 Principal.run(stop_tunnel) 会在设备协调等待前持有
旧global/Ticket；epoch的原clear future也有实际存储。原Replies诊断与
success wire费用不覆盖这份具体F。本项仅在HTTP层，保留原permission→
Replies→selector→success payload前缀后，从同一global bank精确准入
实际stop+Principal F或原clear F，再首次poll。新metadata拒绝是明确的
503/resource_limit，发生在Ticket/表锁/控制mutation前；原响应prefix照常
付费与析构。新增模块http_control复用既有PaidFuture，无新unsafe/task/
bank/限额/时钟或身份替代。

停止仍使用同一个Principal.run围绕同一个core stop，原ScopeDenied到
Response转换移到同一paid F末尾，原inner Anyhow Result和handler错误映射
保持。epoch继续原state.clear()，未新增Principal取消guard。核心stop/
stop_with_ticket、clear、notify_shutdown与server源码和20b6a19原状态完全
一致，有逐字/hash guard；内部authority撤销、close/Drop等控制不加晚额度。
已知gate的原零额度停止、直接clear及终态后clear语义保持。

新增5项验证：

- 实际localhost DELETE/epoch，原published A/Ticket/snapshot保留。测量原
  Replies2048与success wire真实费用，仅留prefix+F N−1，实际coordinator
  或tunnel表锁持有仍先返回503。仅有两份u8 wire backing，零F backing；
  原desired version/publication pointer/Source计数保持，最后全额退款。
- 两份真实stop F layout/地址/整笔扣零；coordinator等待前分别保留不同
  global A/B，clear后闭池0，实际Drop逐份结束原强引用并退各自F费用。
  未把请求取消当作F已结束或声称F支付marker Arc控制块。
- epoch F精确准入后在表锁等待中Drop，原epoch未切换且退回exact F费；
  精确再持压到0并闭池后，直接core clear、notify及终态后clear在零新
  受控backing下仍完成原切换，最后全额退款。
- 原已知published gate在closed0直接core stop成功，原Stopped version/
  device generation更换、FakeDevice实际Drop1、重复stop仍不新增准入，
  原Source次数保持，最终原池全额退款。
- 已配置Principal在coordinator等待/closed0时由真实私有policy reload
  撤销；原403先于mutation，exact F退款，原publication/snapshot/Source1/
  device drop0保持。配置路径为既有私有临时fixture，无真实设备操作。

R1首个strict因新测试观察MutexGuard的显式drop仍被await_holding_lock
检查命中；只将该观察限定在独立词法块，未改生产、断言或增加allow。
round2 全16命令通过，全部Cargo/build由主控串行：

- http-control过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1817 filtered out; finished in 0.05s
- intent过滤：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1811 filtered out; finished in 1.65s
- upgrade-epoch过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1816 filtered out; finished in 0.30s
- shutdown-admission过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1814 filtered out; finished in 2.06s
- lifecycle过滤：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1793 filtered out; finished in 6.71s
- access过滤：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1806 filtered out; finished in 11.66s
- native-stop过滤：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1821 filtered out; finished in 0.01s
- native-fence过滤：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1821 filtered out; finished in 0.02s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1818 filtered out; finished in 0.00s
- cli-full过滤：test result: ok. 1809 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 246.15s

另含strict CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。700冻结源码无漂移，4处差异仅CLI。Core/
Cargo.lock全量hash与20b6a19相同，Core沿用846c533已验29门禁/tunnel159/
owner42/无mDNS37/watch10/无mDNS10，未声称本轮重跑。原core控制正文与
server不变有独立guard。实际host及verified-ios.exe SHA-256：
dcd08f4212873dfdd56a12cb3b60d54a664291e049ec32bedc25df9305d42723
命令日志、source delta/current/提交后rawGit proof及实际二进制归档：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-http-control-frames\round2

F费仅支付实际具体allocation；返回Response/error的独立heap、opaque父
HTTP/框架/task、public Core Arc/Weak/watch、SDK/metadata/config/credential/
transport/allocator/Waker/waiter、原marker根与完整各模式endpoint恢复/
native exposure事务仍属原范围。未豁免或证明finite marker arena容量。
128MiB仍是已准入已知对象池，非整个heap/RSS。未声称Miri/其它平台/真机
验证。主项23交付/6缺开发/L02待全审计、有效30/Q04全部排除不变，仅本地提交。
