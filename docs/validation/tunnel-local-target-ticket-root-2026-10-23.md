# L02 LocalTarget connection Ticket root · 2026-10-23

LocalTarget及upgrade_owner::Local原来inline持for_connection() Ticket，
可在原publication row销毁后独立存活。本项为该捕获准备实际Ticket
根，并把同一根move到原升级callback/F。复用publication实际完整
Strong<Ticket,(PGrant,Origins3)>，整笔G/P及captured global/device真实
foreign-origin费用先准入，随后才clone/project并分配同一个实际根。
原完整layout、覆盖P的真实held permit/count校验及最后physical free
后退款协议保持。没有新unsafe、数值generation、占位费或quota fallback。

连接按原语义不持desired fence：新的borrowed字段选择只包含原
shutdown_generation/generation，第三origin位置None；标准new路径
仍含原intent_version。原origin helper的覆盖、顺序、真实bank去重
和错误体保持，只参数化第三borrowed字段。for_connection显式clone
原gate/generation/shutdown/shutdown_generation四字段、intent_version=None，
不再瞬时clone后丢掉未保留的foreign desired。没有读取新epoch、
新key或最新Ticket；canonical key、设备/全局身份及接收器保持。

get_local_target仍在原同一table guard内先复制PaidRecord，再准备
connection root，最后clone publication Life。根拒绝会析构先前
PaidRecord，经原503/tunnel record copy failed路径返回；没有进入
TCP dial或Source/fallback。LocalTarget与Local只改Ticket字段类型，
原handler、filter顺序、dial deadline、commit、callback.complete及
整个local bridge Future/revocation体没有修改。根不持State、native、
record、publication或旧row反向引用；Life close仍在原row移除时发生。
Local concrete F的实际size按原prepare测量，较小field不代替独立R费用。

新增6项离线验证：

- 三个保留bank G/P/真实origin A逐个N−1与足额closed拒绝，无新
  backing，其他已暂存grant和原引用数准确回退。待丢掉的desired C
  本来closed零余量，不能误向C收费。
- G/P精确两份实际根容量下，只分配一个独立connection layout，
  Ticket位于真实span、原key保持，Clone同根；foreign desired原
  fence刻意不current，connection因正确projection仍current。C
  引用数不增加，connection第三origin为空。clear后原device/global
  fence使连接失效，原row/Ticket/Lifecycle/desired结束后C Weak不能
  upgrade，connection不保留C。最后根free回调仍持G/P/A费用，之后
  准确全退；没有延长旧row根寿命。
- 实际manager G与P分别精确R−1，在PaidRecord已成功复制后拒绝，
  原record费用完整回退，row/device不变，Source/usbmux/paired次数
  保持捕获前基线。该start本来已有USB查询，不误称总调用0。
- 完整LocalTarget捕获在closed零余量下跨clear，原device析构一次、
  Life已关闭且Ticket过期，根地址/key保持。先drop record/Life/State
  后P才仅余R，最后根结束G/P全退；不把Life费误归Ticket。
- 实际localhost WebSocket请求在P R−1返回原503/resource_limit及
  原record-copy消息，实际TCP listener未接受dial，Source查询次数
  保持建连前基线，最后实际HTTP/router owner结束后全额退款。
- 原LocalSource实际native publish后捕获root，clear退休原entry，
  notify/drain得到closed-and-empty、原shutdown_status及真实joins/
  peer严格Ok(0) EOF/task baseline，同时root仍保留同一Ticket/key。
  Record/Life/State结束后P只余R，最后root退款。无真实SDK Source poll。

原manual-live欲改变不杀原连接、stop/clear/shutdown idle TCP终止、
同epoch replacement/prune、late publication rollback、native joins/EOF、
通用callback未调用/返回F/取消/析构unwind与deep typed errors回归仍跑。
record-copy与P同时拒绝的优先组合未专门执行，由完整parent sourceguard
证明record在前；新真实根跨exact Local未调用callback再到未poll F的
组合也未直接执行。旧generic owner与实际running upgrade分别证明
其原协议，没有假称该组合已执行或已全审计opaque框架。

首轮strict Clippy通过，新增6中4通过/2失败，原因是新fixture把start
已有的USB查询次数1误写成0；终态后仅改新测试为捕获前后baseline不变。
生产代码没有为这些失败修改。round1日志保留，不记PASS；round2重新
冻结718源码后39门禁全部通过，所有Cargo/build/fmt/test由主控串行：

- connection-ticket：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1874 filtered out; finished in 0.07s
- upgrade-owner：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1875 filtered out; finished in 0.00s
- upgrade-epoch：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1874 filtered out; finished in 0.32s
- publication-ticket：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1875 filtered out; finished in 0.01s
- publication-all：test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 1868 filtered out; finished in 0.01s
- lifecycle-all：test result: ok. 335 passed; 0 failed; 0 ignored; 0 measured; 1545 filtered out; finished in 86.27s
- native-retirement：test result: ok. 110 passed; 0 failed; 1 ignored; 0 measured; 1769 filtered out; finished in 43.56s
- episode-ticket：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1877 filtered out; finished in 0.00s
- receipt-origin：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1877 filtered out; finished in 0.00s
- registry-all：test result: ok. 47 passed; 0 failed; 0 ignored; 0 measured; 1833 filtered out; finished in 0.18s
- lifecycle：test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 1851 filtered out; finished in 6.77s
- intent-cell：test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1870 filtered out; finished in 0.00s
- manual-success：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1867 filtered out; finished in 0.72s
- global-metadata：test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 1863 filtered out; finished in 0.05s
- reconnect：test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1827 filtered out; finished in 6.87s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.01s
- failure-table：test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 1867 filtered out; finished in 0.11s
- error-payload：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1871 filtered out; finished in 0.03s
- failure-slot：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1873 filtered out; finished in 0.00s
- manager-owner：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1876 filtered out; finished in 0.00s
- http-control：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1875 filtered out; finished in 0.12s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1864 filtered out; finished in 0.04s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1872 filtered out; finished in 0.04s
- cli-full：test result: ok. 1867 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.97s

另含shared weak/identity/origin census/native/intent route/scan/ensure/future/
publication-retain定向门禁、strict CLI all-features/all-targets Clippy
-D warnings、build/fmt/diff，实际只读manager1（Source/usbmux trap0），
host5 PASS/0 FAIL/22 NOT_RUN，real_device_executed=false。
718冻结源码hash全部匹配；9处CLI差异（4生产/5测试，3新增测试文件）。
Core/Cargo.lock与e253/ec25一致，旧Core24证据沿用，本轮未重跑。
独立Agent提供初步源码核查后因使用额度耗尽，未写最终九SHA报告；
本项没有独立最终实现批准。主控复核raw diff和新测试，九SHA与冻结
inventory一致；Agent没有Cargo/源码编辑/commit。主控guard逐体核对原Source/
native/publication/rollback/clock、helper错误、callback/F和分配器协议。
实际host及verified-ios.exe归档SHA-256 f5e38595c2d60989d2cb741bd998e59a07231a973ec20b7e28c6eb54ee4079bb
命令/log/result/frozen/current/source-delta/raw HEAD提交后proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-local-target-ticket-root\round2

本项覆盖LocalTarget→Local实际Ticket根，不代表其它GlobalEpoch/
failed Node/RefreshProof/writer/snapshot等keeper已封口。底层std Arc
physical marker arena、all-retainer/Weak/control峰值/不可变TOTAL，
opaque/public Core/SDK/legacy/framework/task/waker/waiter/allocator/native/
transport及全部mode endpoint恢复/native exposure仍属原scope。
128MiB仍为已知对象准入池、非完整heap/RSS；无Miri/其它平台/真实设备
结论。23交付/6缺开发/L02待全审计、有效30和Q04全部排除保持。
