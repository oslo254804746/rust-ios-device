# L02 WebSocket upgrade owner · 2026-10-22

远端 WebSocket 原 GlobalEpoch、socket、principal 被 Axum 的升级回调和桥接
future 持有，原 HTTP Replies/target/request 临时费用却先结束。该回调在等待
Hyper upgrade 时仍存在，不能用 HTTP handler 费用证明其完整寿命的准入。

新增私有 upgrade_owner：按 Strong<Mutex<Option<WebSocket>>, ()> 完整声明根
与编译器实际具体桥接 F 的尺寸，原 global bank 整笔准入后再创建两个真实
backing。local 在目标表查询前预留，remote 在原 hop/注册验证后、global epoch/
远端查询/请求/拨号前预留。local 无目标时先释放未用预留，再走原 remote 分支。
完整费用不足时返回原 ResourceLimit 503，未查询/拨号或改走 fallback。

原成功 dial、ticket/publication/epoch commit 后，将原 context 与输入根 clone
放入同一个已预付 F；随后才把捕获 paid F 的回调交给 on_upgrade。回调只把
原 socket 放入原槽并返回同一个 F，不晚申请费用、不新建 channel/task。首次
poll 后 socket 移入 F 的实际状态，输入槽可释放；F 的 backing 继续持费到
原 bridge/捕获实际析构及 dealloc。取消未调用回调、未 poll F、正常完成和原
Drop panic 均走既有 Strong/PaidFuture 的实际释放后退款顺序。未新增 unsafe、
bank、固定额度扩张或标准 Arc 私有布局估算。

原 local publication.run(ticket.run(...))、remote epoch.run(...) 与 principal
撤销 select 的正文、日志、原 native IO/elapsed 映射、绝对 timeout 和最终
commit 均保持。没有变更 nonce 身份/清空/停止语义或 native join/EOF 规则。

新增6测试：5个 generic owner 测试及1个真实 HTTP 双分支拒绝测试。N−1
观察零 backing 且 factory 不构造/不 poll；exact 配额在两 backing 前全扣，
64字节对齐 !Unpin F 的真实分配地址在 poll/Drop 不变；closed0 构造/回调
移交/取消没有新受控 backing；原 Box 输入指针、原 typed panic、一次析构、
实际 input free 时费用仍扣及最终退款已验。真实 HTTP local 测试持有实际
目标表写锁，503 在该查询前返回；显式 remote 同样拒绝，唯一受控分配是
2048字节 Replies，Source/usbmux/上游请求均0。原 active remote epoch 测试
额外核对 HTTP/target 临时对象结束后实际 F 费用；未知UID stop 创建的原
canonical suppression gate 依既有真实尺寸独立计费。原双跳 echo、local
stop 保活、clear/shutdown 关闭、TCP 严格 Ok(0)/实际 peer join 保留，并等到
同一 bank 的真实最终 F 退款。未把通知当作 owner 已结束。

只读 agent 检查 pinned Axum0.7.9 on_upgrade 的实际回调保留/调用次序和本实现，
未发现生产缺陷；随后核对新增测试，未发现阻塞缺口。agent 未运行 Cargo 或
编辑仓库。外部 ws-upgrade-owner-design-audit.md/json 包含9个 primary SHA。
主控保留精度备注：pending-callback 的 after-free 上界独立断言较宽，严格
顺序由 unpolled transfer 的 after-free==0 与既有 Strong 原位释放测试覆盖。

生产预检查通过。R1 首个 strict 测试编译因观察 Arc 被移入后又读取失败；
仅修正测试 clone 名。R2 strict/owner5通过，HTTP最后立即全额退款断言失败，
仅补有界等待实际 Hyper/router 最后 owner 结束；R3 owner6/future4/weak4/
upgrade epoch6通过，remote active 公式漏算 stop 未知UID创建的 canonical
gate，仅按既有 actual layout helper补独立费用/寿命。生产保持，未删拒绝/
地址/退款断言或扩固定额度；日志保留。round4 全13命令
通过，全部 Cargo/build 由主控串行执行：

- strict CLI all-features/all-targets Clippy -D warnings。
- upgrade owner：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1794 filtered out; finished in 0.02s
- future storage：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1796 filtered out; finished in 0.00s
- 原 weak root：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1796 filtered out; finished in 0.00s
- 原 upgrade epoch：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1794 filtered out; finished in 0.30s
- 原 remote epoch：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1799 filtered out; finished in 0.02s
- 授权/撤销：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1784 filtered out; finished in 12.26s
- 完整 CLI：test result: ok. 1787 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 251.33s
- build/fmt/diff、实际只读 manager1（Source/usbmux trap0）、host5 PASS /
  0 FAIL / 22 NOT_RUN，real_device_executed=false。

695冻结源码无漂移，6处仅CLI差异；所有 Core/Cargo.lock 哈希与5bface3已验
状态相同，沿用846c533 Core tunnel159/owner42/无mDNS37/watch10/无mDNS10及
29门禁，未声称本轮重跑Core。实际 host 和归档 verified-ios.exe SHA-256：
2acaa8035c201f01c454aeb94dbc47f681bd5db74ef5e57aa61e894e0a38fd09
current/source-delta/提交后 rawGit proof 与命令日志：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-ws-upgrade-owner\round4

本项覆盖声明的输入根和具体 future 捕获；Axum/Hyper 任务单元、transport/
TLS/WebSocket 私有 buffer、credential/principal 深层 heap、header、allocator/
Waker/waiter、原 Arc generation、opaque error/report、Core公开/SDK metadata
及完整各模式 endpoint恢复/native exposure事务仍属原待办范围，未豁免。
该修复只补上 marker retainer 普查的一处必要条件，不构成有限 arena 容量
证明。128MiB仍是已准入已知对象池，非整个heap/RSS；未声称Miri/其它平台/
真实设备验证。主项23交付/6缺开发/L02待全审计、有效30/Q04全部排除不变。
仅本地提交，继续原七主项。
