# L02 complete scan frame owner · 2026-10-22

refresh_once 原本在等待 scan_lock 前就持有 ScanControl/全局与扫描 epoch，
实际外层扫描 frame 未独立准入；Outcomes session 在取得锁后才创建，不能
覆盖此前的 frame/capture 寿命。本项先执行原 whole Deadline 构造，再通过
scan_deadline 执行原 terminal 检查和 discovery deadline checked_add；此
前缀不捕获 generation/receiver。随后从同一原 global bank 为编译器实际
refresh_admitted F 整笔申请精确 backing，复用 PaidFuture 的原 Box/pin。
首次 paid F poll 才执行原 scan_control_at 捕获，之后原 epoch clone、
whole.run(control.drive(scan_lock))、Outcomes.begin、whole.run(refresh_scan)
与 Session.finish 正文保持。直接 scan_control(timeout) 测试入口仍共享
原前缀与 capture；未引入任务、unsafe、新 bank、限额扩张或时钟重启。

新增的是明确的 pre-control/pre-Source ResourceLimit 分支：新 F 额度不足，
包括 whole 已准备为零/过期时，返回准入拒绝，不捕获控制或生成一个从未
选中的 timeout。原 whole overflow、已 terminal 和 discovery overflow
仍先于这次准入；已准入且期限为零/过期仍由原 timer 返回 native Elapsed。
已取得 Source/实际 timer 原因继续原 Session/receipt/cancel/cleanup 合同。
原 docs/error-contract 2073/2075、scan-control及Source前准入合同已经逐条
核对；此前额外推断“新额度拒绝也必须复现假想旧超时/取消”的限制没有原
合同依据，已明确撤回。没有用未经准入的 marker/timer fallback 绕过新门禁。
权威出处、15个指纹及矩阵在外部 scan-frame-prefix-resolution.md/json。

新增5项测试覆盖：

- six-case whole overflow/terminal/discovery overflow/有效或零 whole下N−1/
  有效前缀闭池0矩阵，零 frame backing/capture/lock/session/Source。
- 同时两个真实 F backing 的 layout/地址/原池精确扣费；first 捕获 A 后
  clear，second 捕获不同 B 后再 clear。闭池0时请求取消不退款，first实际
  Drop只结束A及一份F费，B继续；second Drop才结束B和另一份F费。
- 在实际 F allocation hook 延迟60ms，分别耗尽原20ms whole/discovery
  绝对期限，原 native Elapsed 与 Source0证明两钟未移后/重启。
- 实际 pending USB Source+Session caller Drop，闭池时 Source退出、session
  归零、scan锁释放、原 F/其它实际存储全额退款。
- 实际 HTTP POST refresh，持有原 scan lock，预留响应费用+F N−1，classified
  502 ResourceLimit在锁/Source工作前返回，最后HTTP/router owner全额退款。

R1 strict/新frame5/setup19/whole7/terminal4/shutdown8通过，global metadata
12通过/5失败：旧精确夹具未给新增F留费用而提前拒绝或漏算其最终退款。
主控按实际 scan_frame_bytes 给十个相关前缀/退款公式加费，另核对 Source0
本来就通过的早期用例，保持其原目标拒绝点。没有增加固定池或删除原N−1/
Source次数/identity/backoff/zero balance/原IO/refund断言。按只读review建议，
同一旧marker pair测试加强为不同A/B两代，并补有效前缀closed0矩阵；生产
自R1起保持。只读agent未运行Cargo/修改仓库，审查与指纹另在外部
scan-frame-implementation-audit.md/json；不能将静态意见视作运行PASS。

round2 全17命令通过，所有Cargo/build主控串行：

- scan-frame过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1801 filtered out; finished in 0.14s
- scan-setup过滤：test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 1787 filtered out; finished in 2.39s
- scan-total过滤：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1799 filtered out; finished in 0.39s
- scan-terminal过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1802 filtered out; finished in 0.01s
- shutdown-admission过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1798 filtered out; finished in 2.06s
- global-metadata过滤：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1789 filtered out; finished in 0.04s
- bookkeeping过滤：test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 1783 filtered out; finished in 5.62s
- discovery-future过滤：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1800 filtered out; finished in 0.02s
- server过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1798 filtered out; finished in 10.32s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1802 filtered out; finished in 0.00s
- cli-full过滤：test result: ok. 1793 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 236.07s

另含CLI all-features/all-targets Clippy -D warnings、build/fmt/diff、实际只读
manager1（Source/usbmux trap0）、host5 PASS/0 FAIL/22 NOT_RUN，
real_device_executed=false。Core/Cargo.lock所有hash与419bcda已验状态相同，
本轮仅CLI，Core沿用846c533的29门禁/tunnel159/owner42/无mDNS37/watch10/
无mDNS10，未声称重跑。696冻结源码无漂移；差异见精确source-delta。
实际host及归档verified-ios.exe SHA-256：
18c1a22f2459b1ce2785548fdb78fa28daaa7454dea46fdc9dba326b4a7c0146
全部命令日志、source delta/current/rawGit提交后proof及二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-scan-frame-owner\round2

原 scan/global/device/desired marker自身根、opaque父HTTP/monitor/task cell、
error/report/SDK metadata/config/credential/transport/allocator/Waker/waiter、
Core公开native Arc/Weak/watch及完整各模式endpoint恢复/native exposure事务
仍属原范围。F费不覆盖独立返回error/metadata堆，未据此证明有限marker arena。
128MiB为已准入已知对象池，非整个heap/RSS；未声称Miri/其它平台/真机验证。
L02仍待全审计，23交付/6缺开发/L02待核、有效30/Q04全部排除不变。仅本地提交。
